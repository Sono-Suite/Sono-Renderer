//! Standalone, file-based video composition. This module deliberately has no
//! dependency on Watch, Sonolus engines, LevelData, or gameplay configuration.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub const COMPOSITION_FORMAT: &str = "sono-renderer-composition";
pub const COMPOSITION_SCHEMA_VERSION: u32 = 21;
const COMPOSITION_FILE_LIMIT: u64 = 32 * 1024 * 1024;
const COMPOSITION_TEMPLATE_FORMAT: &str = "sono-renderer-presentation-template";
const COMPOSITION_TEMPLATE_SCHEMA_VERSION: u32 = 1;
const COMPOSITION_TEMPLATE_FILE_LIMIT: u64 = 16 * 1024 * 1024;
const MAX_TEMPLATE_SOURCE_SLOTS: usize = 128;
const MAX_TEMPLATE_CLIPS: usize = 8_192;
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

mod portable_path {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::PathBuf;

    pub fn serialize<S>(path: &PathBuf, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&path.to_string_lossy().replace('\\', "/"))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        // Forward slashes are valid separators on Windows and native separators
        // on Unix. Keeping the serialized spelling intact preserves relative
        // paths on both platforms.
        String::deserialize(deserializer).map(PathBuf::from)
    }
}

mod optional_portable_path {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::PathBuf;

    pub fn serialize<S>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match path {
            Some(path) => serializer.serialize_some(&path.to_string_lossy().replace('\\', "/")),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer).map(|path| path.map(PathBuf::from))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    Video,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GeneratedShapeKind {
    Rectangle,
    Ellipse,
    Line,
}

/// Filled primitive geometry stored as normalized bounds on its generated
/// source canvas. It stays in the regular media/clip path for transform,
/// opacity, ordering, and animation behavior.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct GeneratedShape {
    pub kind: GeneratedShapeKind,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Default for GeneratedShape {
    fn default() -> Self {
        Self {
            kind: GeneratedShapeKind::Rectangle,
            x: 0.1,
            y: 0.1,
            width: 0.8,
            height: 0.8,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MediaSource {
    pub id: String,
    #[serde(with = "portable_path")]
    pub path: PathBuf,
    #[serde(default, with = "optional_portable_path")]
    pub nested_composition_path: Option<PathBuf>,
    #[serde(default, with = "optional_portable_path")]
    pub proxy_path: Option<PathBuf>,
    /// A proxy is valid only for the media source revision from which it was
    /// generated, and only while its own bytes still match this record.
    #[serde(default)]
    pub proxy_metadata: Option<VideoProxyMetadata>,
    pub display_name: String,
    pub has_video: bool,
    pub has_audio: bool,
    pub duration: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// True for a single-frame raster source. Its source duration is unbounded;
    /// each timeline instance supplies the actual duration.
    #[serde(default)]
    pub still_image: bool,
    /// Generated sources have no filesystem path. The value is an RGB hex color.
    #[serde(default)]
    pub solid_color: Option<String>,
    /// Optional filled primitive drawn on the generated solid source canvas.
    #[serde(default)]
    pub generated_shape: Option<GeneratedShape>,
    /// Bounded peak summary for the shared media browser/timeline display.
    /// Empty means unavailable; it is never used as source audio.
    #[serde(default)]
    pub waveform_peaks: Vec<f32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for MediaSource {
    fn default() -> Self {
        Self {
            id: String::new(),
            path: PathBuf::new(),
            nested_composition_path: None,
            proxy_path: None,
            proxy_metadata: None,
            display_name: String::new(),
            has_video: false,
            has_audio: false,
            duration: 0.0,
            width: None,
            height: None,
            still_image: false,
            solid_color: None,
            generated_shape: None,
            waveform_peaks: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct MediaFileSignature {
    pub length: u64,
    pub modified_unix_nanos: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VideoProxyMetadata {
    pub source_id: String,
    pub source_signature: MediaFileSignature,
    pub proxy_signature: MediaFileSignature,
    pub width: u32,
    pub height: u32,
    pub duration: f64,
    pub video_codec: String,
    pub generated_by_renderer: bool,
}

impl Default for VideoProxyMetadata {
    fn default() -> Self {
        Self {
            source_id: String::new(),
            source_signature: MediaFileSignature::default(),
            proxy_signature: MediaFileSignature::default(),
            width: 0,
            height: 0,
            duration: 0.0,
            video_codec: String::new(),
            generated_by_renderer: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoProxyStatus {
    Missing,
    Ready,
    ReadyOriginalOffline,
    ProxyOffline,
    Stale,
    MetadataMissing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipKind {
    Video,
    Audio,
    Text,
    Adjustment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TextAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Provenance for timed caption text stored as an ordinary text clip. The clip's
/// `timeline_start`, `duration`, and `text` remain the authoritative editable cue;
/// this metadata only describes where those values came from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CaptionOrigin {
    Imported,
    Transcribed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CaptionMetadata {
    pub origin: CaptionOrigin,
    pub source_id: Option<String>,
    pub source_clip_id: Option<String>,
    pub language: Option<String>,
    pub model_id: Option<String>,
    /// Only populated when a selected backend actually reports a score with a
    /// defined confidence interpretation. Whisper segment probabilities are not
    /// treated as calibrated confidence, so this is normally `None`.
    pub confidence: Option<f64>,
    /// Optional clip-local word boundaries. Segment-only backends leave this
    /// empty rather than distributing cue duration across words artificially.
    pub words: Vec<CaptionWordTiming>,
}

impl Default for CaptionMetadata {
    fn default() -> Self {
        Self {
            origin: CaptionOrigin::Imported,
            source_id: None,
            source_clip_id: None,
            language: None,
            model_id: None,
            confidence: None,
            words: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CaptionWordTiming {
    /// Time relative to the containing text clip, in seconds.
    pub start: f64,
    pub end: f64,
    pub text: String,
}

impl Default for ClipKind {
    fn default() -> Self {
        Self::Video
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Easing {
    #[default]
    Linear,
    Hold,
    EaseIn,
    EaseOut,
    EaseInOut,
}

impl Easing {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Hold => "Hold",
            Self::EaseIn => "Ease in",
            Self::EaseOut => "Ease out",
            Self::EaseInOut => "Ease in/out",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrackKind {
    Visual,
    Audio,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionTrack {
    pub id: String,
    pub name: String,
    pub kind: TrackKind,
    pub order: i32,
    pub enabled: bool,
    pub muted: bool,
    pub solo: bool,
    /// Constant linear gain applied to every audio clip routed through this track.
    #[serde(default = "default_track_gain")]
    pub gain: f64,
    #[serde(default)]
    pub locked: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl CompositionTrack {
    fn new(id: &str, name: &str, kind: TrackKind, order: i32) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            kind,
            order,
            enabled: true,
            muted: false,
            solo: false,
            gain: 1.0,
            locked: false,
            extra: BTreeMap::new(),
        }
    }
}

fn default_track_gain() -> f64 {
    1.0
}

impl Default for CompositionTrack {
    fn default() -> Self {
        Self::new("", "Track", TrackKind::Visual, 0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AnimatedProperty {
    X,
    Y,
    ScaleX,
    ScaleY,
    Rotation,
    Opacity,
    CropLeft,
    CropTop,
    CropRight,
    CropBottom,
    Blur,
    MaskX,
    MaskY,
    MaskWidth,
    MaskHeight,
    MaskFeather,
    Gain,
    Pan,
    FontSize,
    OutlineWidth,
    ShadowX,
    ShadowY,
}

/// Supported serializable effects for a visual source. Effect instances are
/// evaluated in their order in `CompositionClip::effects`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum VisualEffectKind {
    GaussianBlur {
        sigma: f64,
    },
    Brightness {
        amount: f64,
    },
    Contrast {
        amount: f64,
    },
    Saturation {
        amount: f64,
    },
    Hue {
        degrees: f64,
    },
    Sharpen {
        amount: f64,
    },
    Gamma {
        amount: f64,
    },
    ChromaKey {
        key_color: String,
        tolerance: f64,
        softness: f64,
    },
}

/// Scalar parameters that can be animated by the existing timeline key model.
/// Only FFmpeg filters with frame-evaluated expressions are exposed here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EffectParameter {
    Amount,
    HueDegrees,
}

impl EffectParameter {
    pub fn label(self) -> &'static str {
        match self {
            Self::Amount => "Amount",
            Self::HueDegrees => "Hue degrees",
        }
    }
}

impl VisualEffectKind {
    pub fn label(&self) -> &'static str {
        match self {
            Self::GaussianBlur { .. } => "Gaussian blur",
            Self::Brightness { .. } => "Brightness",
            Self::Contrast { .. } => "Contrast",
            Self::Saturation { .. } => "Saturation",
            Self::Hue { .. } => "Hue",
            Self::Sharpen { .. } => "Sharpen",
            Self::Gamma { .. } => "Gamma",
            Self::ChromaKey { .. } => "Chroma key",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VisualEffectInstance {
    pub id: String,
    pub enabled: bool,
    pub kind: VisualEffectKind,
    #[serde(default)]
    pub animated_parameters: BTreeMap<EffectParameter, Vec<ScalarKeyframe>>,
}

impl VisualEffectInstance {
    pub fn new(kind: VisualEffectKind) -> Self {
        Self {
            id: unique_id("effect"),
            enabled: true,
            kind,
            animated_parameters: BTreeMap::new(),
        }
    }
}

impl Default for VisualEffectInstance {
    fn default() -> Self {
        Self::new(VisualEffectKind::GaussianBlur { sigma: 4.0 })
    }
}

impl Default for VisualEffectKind {
    fn default() -> Self {
        Self::GaussianBlur { sigma: 4.0 }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum VisualMaskShape {
    #[default]
    Rectangle,
    Ellipse,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TransitionKind {
    #[default]
    CrossDissolve,
    DipToBlack,
    DipToWhite,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CompositionExportPreset {
    /// Fast preview-quality delivery, useful for review copies.
    Draft,
    /// Historical default retained for existing projects and exports.
    #[default]
    Balanced,
    /// Higher quality at the cost of encode time and file size.
    HighQuality,
}

impl CompositionExportPreset {
    pub fn label(self) -> &'static str {
        match self {
            Self::Draft => "Draft (fast)",
            Self::Balanced => "Balanced",
            Self::HighQuality => "High quality",
        }
    }

    fn encoding(self) -> (u8, &'static str, &'static str) {
        match self {
            Self::Draft => (26, "veryfast", "128k"),
            Self::Balanced => (18, "medium", "192k"),
            Self::HighQuality => (16, "slow", "256k"),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SourceTimeMode {
    #[default]
    Forward,
    Reverse,
    Freeze,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionTransition {
    pub id: String,
    pub from_clip_id: String,
    pub to_clip_id: String,
    pub kind: TransitionKind,
    #[serde(default)]
    pub easing: Easing,
    /// Composition timeline placement, in seconds.
    pub start_time: f64,
    pub duration: f64,
}

impl Default for CompositionTransition {
    fn default() -> Self {
        Self {
            id: String::new(),
            from_clip_id: String::new(),
            to_clip_id: String::new(),
            kind: TransitionKind::CrossDissolve,
            easing: Easing::Linear,
            start_time: 0.0,
            duration: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VisualMask {
    pub shape: VisualMaskShape,
    /// Normalized bounds in the cropped/scaled clip image before rotation or positioning.
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    /// Normalized softness relative to the short clip edge. Zero is a hard edge.
    pub feather: f64,
    pub inverted: bool,
}

impl Default for VisualMask {
    fn default() -> Self {
        Self {
            shape: VisualMaskShape::Rectangle,
            x: 0.1,
            y: 0.1,
            width: 0.8,
            height: 0.8,
            feather: 0.0,
            inverted: false,
        }
    }
}

impl AnimatedProperty {
    pub fn label(self) -> &'static str {
        match self {
            Self::X => "X",
            Self::Y => "Y",
            Self::ScaleX => "Scale X",
            Self::ScaleY => "Scale Y",
            Self::Rotation => "Rotation",
            Self::Opacity => "Opacity",
            Self::CropLeft => "Crop Left",
            Self::CropTop => "Crop Top",
            Self::CropRight => "Crop Right",
            Self::CropBottom => "Crop Bottom",
            Self::Blur => "Blur",
            Self::MaskX => "Mask X",
            Self::MaskY => "Mask Y",
            Self::MaskWidth => "Mask Width",
            Self::MaskHeight => "Mask Height",
            Self::MaskFeather => "Mask Feather",
            Self::Gain => "Gain",
            Self::Pan => "Pan",
            Self::FontSize => "Font Size",
            Self::OutlineWidth => "Outline Width",
            Self::ShadowX => "Shadow X",
            Self::ShadowY => "Shadow Y",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ScalarKeyframe {
    /// Composition timeline time in seconds.
    pub time: f64,
    pub value: f64,
    pub easing_to_next: Easing,
}

impl Default for ScalarKeyframe {
    fn default() -> Self {
        Self {
            time: 0.0,
            value: 0.0,
            easing_to_next: Easing::Linear,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PositionKeyframe {
    /// Composition timeline time in seconds.
    pub time: f64,
    pub x: f64,
    pub easing_to_next: Easing,
}

impl Default for PositionKeyframe {
    fn default() -> Self {
        Self {
            time: 0.0,
            x: 0.0,
            easing_to_next: Easing::Linear,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionClip {
    pub id: String,
    pub name: String,
    pub kind: ClipKind,
    pub source_id: Option<String>,
    /// Stable identity of a compatible visual or audio track. `None` is
    /// accepted only as a legacy/constructed value and resolves to the first
    /// compatible track during save and migration.
    #[serde(default)]
    pub track_id: Option<String>,
    /// Clips sharing this ID are edited as one linked A/V instance.
    #[serde(default)]
    pub link_group_id: Option<String>,
    /// User-created edit group. This is independent from synchronized A/V links.
    #[serde(default)]
    pub group_id: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Timeline start, in composition seconds.
    pub timeline_start: f64,
    /// Source in point, in source seconds.
    pub source_start: f64,
    pub duration: f64,
    /// Source-time speed. `duration` remains timeline duration; the source
    /// window is `duration * playback_rate` except in Freeze mode.
    #[serde(default = "default_playback_rate")]
    pub playback_rate: f64,
    #[serde(default)]
    pub source_time_mode: SourceTimeMode,
    pub z_order: i32,
    /// Top-left position in output pixels.
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    #[serde(default)]
    pub rotation_degrees: f64,
    /// Fractions cropped from each source edge, in 0..1.
    pub crop_left: f64,
    pub crop_top: f64,
    pub crop_right: f64,
    pub crop_bottom: f64,
    pub opacity: f64,
    /// Gaussian blur sigma in source pixels; zero disables blur.
    pub blur: f64,
    /// Ordered effects applied after the legacy Gaussian-blur property and
    /// before clip opacity/compositing.
    #[serde(default)]
    pub effects: Vec<VisualEffectInstance>,
    #[serde(default)]
    pub mask: Option<VisualMask>,
    /// Audio routing is separate from visual duplication. Video instances
    /// start muted; users can add one explicit audio clip from the same source.
    pub include_audio: bool,
    pub gain: f64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub solo: bool,
    /// Stereo balance in -1 (left) to 1 (right); mono sources are duplicated
    /// into the stereo composition mix before balance is applied.
    #[serde(default)]
    pub pan: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub text: String,
    /// Timed captions are still normal text clips. Their text and timing live
    /// in the existing clip fields; this stores provenance only.
    #[serde(default)]
    pub caption: Option<CaptionMetadata>,
    #[serde(default, with = "optional_portable_path")]
    pub font_path: Option<PathBuf>,
    #[serde(default)]
    pub text_alignment: TextAlignment,
    #[serde(default)]
    pub text_background_enabled: bool,
    #[serde(default = "default_text_background_color")]
    pub text_background_color: String,
    #[serde(default = "default_text_background_opacity")]
    pub text_background_opacity: f64,
    #[serde(default = "default_text_background_padding")]
    pub text_background_padding: u32,
    pub font_size: u32,
    pub text_color: String,
    pub outline_color: String,
    pub outline_width: f64,
    pub shadow_x: f64,
    pub shadow_y: f64,
    pub position_keyframes: Vec<PositionKeyframe>,
    /// Canonical scalar animation store. The legacy X-only list is retained
    /// for reading old documents and is migrated into this map on load.
    #[serde(default)]
    pub animated_properties: BTreeMap<AnimatedProperty, Vec<ScalarKeyframe>>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for CompositionClip {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            kind: ClipKind::Video,
            source_id: None,
            track_id: None,
            link_group_id: None,
            group_id: None,
            enabled: true,
            timeline_start: 0.0,
            source_start: 0.0,
            duration: 1.0,
            playback_rate: 1.0,
            source_time_mode: SourceTimeMode::Forward,
            z_order: 0,
            x: 0.0,
            y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation_degrees: 0.0,
            crop_left: 0.0,
            crop_top: 0.0,
            crop_right: 0.0,
            crop_bottom: 0.0,
            opacity: 1.0,
            blur: 0.0,
            effects: Vec::new(),
            mask: None,
            include_audio: false,
            gain: 1.0,
            muted: false,
            solo: false,
            pan: 0.0,
            fade_in: 0.0,
            fade_out: 0.0,
            text: String::new(),
            caption: None,
            font_path: None,
            text_alignment: TextAlignment::Left,
            text_background_enabled: false,
            text_background_color: default_text_background_color(),
            text_background_opacity: default_text_background_opacity(),
            text_background_padding: default_text_background_padding(),
            font_size: 64,
            text_color: "FFFFFF".into(),
            outline_color: "000000".into(),
            outline_width: 3.0,
            shadow_x: 2.0,
            shadow_y: 2.0,
            position_keyframes: Vec::new(),
            animated_properties: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }
}

fn source_start_for_subclip(
    original: &CompositionClip,
    timeline_offset: f64,
    timeline_duration: f64,
    source_is_unbounded: bool,
) -> f64 {
    if source_is_unbounded {
        return 0.0;
    }
    match original.source_time_mode {
        SourceTimeMode::Forward => original.source_start + timeline_offset * original.playback_rate,
        SourceTimeMode::Reverse => {
            original.source_start
                + (original.duration - timeline_offset - timeline_duration) * original.playback_rate
        }
        SourceTimeMode::Freeze => original.source_start,
    }
}

impl CompositionClip {
    pub fn animated_value(&self, property: AnimatedProperty, time: f64) -> f64 {
        let base = base_property_value(self, property);
        if let Some(keys) = self.animated_properties.get(&property) {
            evaluate_keyframes(keys, time, base)
        } else if property == AnimatedProperty::X && !self.position_keyframes.is_empty() {
            evaluate_legacy_x(&self.position_keyframes, time, base)
        } else {
            base
        }
    }

    pub fn has_animation(&self, property: AnimatedProperty) -> bool {
        self.animated_properties
            .get(&property)
            .is_some_and(|keys| !keys.is_empty())
            || (property == AnimatedProperty::X && !self.position_keyframes.is_empty())
    }

    pub fn set_property_at(
        &mut self,
        property: AnimatedProperty,
        time: f64,
        value: f64,
    ) -> Result<()> {
        finite_range(time, 0.0, 86_400.0, "keyframe time")?;
        let (min, max) = animated_property_range(property);
        finite_range(value, min, max, "property value")?;
        ensure_mask_for_property(self, property);
        if self.has_animation(property) {
            self.migrate_legacy_x_animation();
            let keys = self.animated_properties.entry(property).or_default();
            if let Some(existing) = keys
                .iter_mut()
                .find(|key| (key.time - time).abs() < 0.000_001)
            {
                existing.value = value;
            } else {
                keys.push(ScalarKeyframe {
                    time,
                    value,
                    easing_to_next: Easing::Linear,
                });
                keys.sort_by(|a, b| a.time.total_cmp(&b.time));
            }
        } else {
            set_base_property_value(self, property, value);
        }
        Ok(())
    }

    pub fn add_keyframe(
        &mut self,
        property: AnimatedProperty,
        time: f64,
        value: f64,
        easing: Easing,
    ) -> Result<()> {
        let (min, max) = animated_property_range(property);
        finite_range(time, 0.0, 86_400.0, "keyframe time")?;
        finite_range(value, min, max, "keyframe value")?;
        ensure_mask_for_property(self, property);
        self.migrate_legacy_x_animation();
        let keys = self.animated_properties.entry(property).or_default();
        if keys.iter().any(|key| (key.time - time).abs() < 0.000_001) {
            bail!(
                "a {} keyframe already exists at this time",
                property.label()
            );
        }
        keys.push(ScalarKeyframe {
            time,
            value,
            easing_to_next: easing,
        });
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        Ok(())
    }

    pub fn remove_keyframe(&mut self, property: AnimatedProperty, time: f64) -> bool {
        self.migrate_legacy_x_animation();
        let (removed, empty) = match self.animated_properties.get_mut(&property) {
            Some(keys) => {
                let old_len = keys.len();
                keys.retain(|key| (key.time - time).abs() >= 0.000_001);
                (keys.len() != old_len, keys.is_empty())
            }
            None => return false,
        };
        if empty {
            self.animated_properties.remove(&property);
        }
        removed
    }

    fn migrate_legacy_x_animation(&mut self) {
        if !self.position_keyframes.is_empty()
            && !self.animated_properties.contains_key(&AnimatedProperty::X)
        {
            self.animated_properties.insert(
                AnimatedProperty::X,
                self.position_keyframes
                    .iter()
                    .map(|key| ScalarKeyframe {
                        time: key.time,
                        value: key.x,
                        easing_to_next: key.easing_to_next,
                    })
                    .collect(),
            );
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_text_background_color() -> String {
    "000000".into()
}

fn default_text_background_opacity() -> f64 {
    0.65
}

fn default_text_background_padding() -> u32 {
    12
}

fn default_playback_rate() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionProject {
    pub format: String,
    pub schema_version: u32,
    pub composition_id: String,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration: f64,
    #[serde(default)]
    pub export_range: Option<CompositionRange>,
    #[serde(default)]
    pub export_preset: CompositionExportPreset,
    /// Proxies are opt-in for preview only. Final exports always use originals.
    #[serde(default)]
    pub use_proxies_for_preview: bool,
    pub title_presets: Vec<TitlePreset>,
    pub sources: Vec<MediaSource>,
    #[serde(default)]
    pub tracks: Vec<CompositionTrack>,
    pub clips: Vec<CompositionClip>,
    #[serde(default)]
    pub markers: Vec<CompositionMarker>,
    #[serde(default)]
    pub transitions: Vec<CompositionTransition>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionRange {
    pub start: f64,
    pub end: f64,
}

/// A project-local reusable text style. It stores ordinary text-clip
/// presentation values; applying one creates a regular editable text clip.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct TitlePreset {
    pub id: String,
    pub name: String,
    pub sample_text: String,
    /// Normalized canvas coordinates so a preset adapts to composition size.
    pub x: f64,
    pub y: f64,
    pub font_size: u32,
    pub text_color: String,
    pub outline_color: String,
    pub outline_width: f64,
    pub shadow_x: f64,
    pub shadow_y: f64,
    pub text_alignment: TextAlignment,
    pub text_background_enabled: bool,
    pub text_background_color: String,
    pub text_background_opacity: f64,
    pub text_background_padding: u32,
    pub built_in: bool,
}

impl Default for TitlePreset {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            sample_text: "Title".into(),
            x: 0.5,
            y: 0.25,
            font_size: 96,
            text_color: "FFFFFF".into(),
            outline_color: "101010".into(),
            outline_width: 4.0,
            shadow_x: 2.0,
            shadow_y: 3.0,
            text_alignment: TextAlignment::Center,
            text_background_enabled: false,
            text_background_color: "000000".into(),
            text_background_opacity: 0.65,
            text_background_padding: 16,
            built_in: false,
        }
    }
}

/// A portable, reusable layout made only from ordinary CompositionProject
/// objects. Applying a template copies tracks, clips and transitions into an
/// editable project; it never introduces another renderer or timeline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionTemplate {
    pub format: String,
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub base_width: u32,
    pub base_height: u32,
    pub fps: u32,
    pub duration: f64,
    pub responsive: bool,
    pub source_slots: Vec<TemplateSourceSlot>,
    pub text_parameters: Vec<TemplateTextParameter>,
    /// Video instances that filled the source template canvas are adapted to
    /// cover the target canvas while preserving source aspect ratio.
    pub canvas_fill_clip_ids: Vec<String>,
    /// Procedural solid/shape sources have no external media dependency and
    /// are included directly in the template document.
    pub generated_sources: Vec<MediaSource>,
    pub tracks: Vec<CompositionTrack>,
    pub clips: Vec<CompositionClip>,
    pub transitions: Vec<CompositionTransition>,
    pub markers: Vec<CompositionMarker>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for CompositionTemplate {
    fn default() -> Self {
        Self {
            format: COMPOSITION_TEMPLATE_FORMAT.into(),
            schema_version: COMPOSITION_TEMPLATE_SCHEMA_VERSION,
            id: unique_id("template"),
            name: String::new(),
            base_width: 1920,
            base_height: 1080,
            fps: 30,
            duration: 30.0,
            responsive: true,
            source_slots: Vec::new(),
            text_parameters: Vec::new(),
            canvas_fill_clip_ids: Vec::new(),
            generated_sources: Vec::new(),
            tracks: Vec::new(),
            clips: Vec::new(),
            transitions: Vec::new(),
            markers: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

/// A media input expected by a template. Slots contain no source paths, so
/// applying the same template to another project requires an explicit binding.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TemplateSourceSlot {
    pub id: String,
    pub label: String,
    pub requires_video: bool,
    pub requires_audio: bool,
    pub still_image: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for TemplateSourceSlot {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: String::new(),
            requires_video: false,
            requires_audio: false,
            still_image: false,
            width: None,
            height: None,
            extra: BTreeMap::new(),
        }
    }
}

/// Text in a template may expose simple values with `{{param:name}}` tokens.
/// Substitution happens once when native clips are copied into a project.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TemplateTextParameter {
    pub name: String,
    pub default_value: String,
}

fn default_title_presets() -> Vec<TitlePreset> {
    vec![
        TitlePreset {
            id: "builtin-centered-title".into(),
            name: "Centered title".into(),
            sample_text: "Your Title".into(),
            y: 0.24,
            font_size: 96,
            built_in: true,
            ..Default::default()
        },
        TitlePreset {
            id: "builtin-lower-third".into(),
            name: "Lower third".into(),
            sample_text: "Name\nSubtitle".into(),
            x: 0.08,
            y: 0.58,
            font_size: 58,
            text_alignment: TextAlignment::Left,
            text_background_enabled: true,
            text_background_color: "101820".into(),
            text_background_opacity: 0.78,
            text_background_padding: 18,
            built_in: true,
            ..Default::default()
        },
        TitlePreset {
            id: "builtin-subtitle-banner".into(),
            name: "Subtitle banner".into(),
            sample_text: "Subtitle".into(),
            y: 0.88,
            font_size: 48,
            text_color: "FFFFFF".into(),
            text_background_enabled: true,
            text_background_color: "000000".into(),
            text_background_opacity: 0.72,
            text_background_padding: 14,
            built_in: true,
            ..Default::default()
        },
        TitlePreset {
            id: "builtin-short-form-caption".into(),
            name: "Short-form caption".into(),
            sample_text: "Caption".into(),
            y: 0.72,
            font_size: 62,
            text_color: "FFFFFF".into(),
            outline_color: "101010".into(),
            outline_width: 5.0,
            shadow_x: 2.0,
            shadow_y: 3.0,
            text_alignment: TextAlignment::Center,
            text_background_enabled: true,
            text_background_color: "101010".into(),
            text_background_opacity: 0.48,
            text_background_padding: 12,
            built_in: true,
            ..Default::default()
        },
    ]
}

impl CompositionTemplate {
    /// Capture the current timeline as a portable template. Imported media is
    /// replaced by source slots; only path-free procedural sources are kept.
    pub fn from_project(project: &CompositionProject, name: &str) -> Result<Self> {
        let mut normalized_project = project.clone();
        normalized_project.normalize_tracks();
        normalized_project.validate()?;
        let project = &normalized_project;
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 128 {
            bail!("template name must contain 1..=128 characters");
        }
        if project.clips.len() > MAX_TEMPLATE_CLIPS {
            bail!("template exceeds the {MAX_TEMPLATE_CLIPS}-clip limit");
        }
        if project.clips.iter().any(|clip| clip.font_path.is_some()) {
            bail!("portable templates cannot embed machine-specific font paths; clear custom font paths or install the font with the template on each machine");
        }

        let mut source_usage = BTreeMap::<&str, (bool, bool)>::new();
        for clip in &project.clips {
            let Some(source_id) = clip.source_id.as_deref() else {
                continue;
            };
            let usage = source_usage.entry(source_id).or_default();
            match clip.kind {
                ClipKind::Video => {
                    usage.0 = true;
                    usage.1 |= clip.include_audio;
                }
                ClipKind::Audio => usage.1 = true,
                ClipKind::Text | ClipKind::Adjustment => {}
            }
        }
        let mut source_slots = Vec::new();
        let mut generated_sources = Vec::new();
        let mut source_replacements = HashMap::new();
        for (source_id, (requires_video, requires_audio)) in source_usage {
            let source = project
                .sources
                .iter()
                .find(|source| source.id == source_id)
                .with_context(|| {
                    format!("template clip references missing source {source_id:?}")
                })?;
            if source.nested_composition_path.is_some() {
                bail!("portable templates do not embed nested-composition paths; flatten or replace that source first");
            }
            if source.solid_color.is_some() {
                let mut generated = source.clone();
                generated.id = unique_id("template-solid");
                generated.path.clear();
                generated.proxy_path = None;
                generated.proxy_metadata = None;
                generated.waveform_peaks.clear();
                source_replacements.insert(source_id.to_owned(), generated.id.clone());
                generated_sources.push(generated);
            } else {
                if source_slots.len() >= MAX_TEMPLATE_SOURCE_SLOTS {
                    bail!("template exceeds the {MAX_TEMPLATE_SOURCE_SLOTS}-source-slot limit");
                }
                let slot = TemplateSourceSlot {
                    id: unique_id("source-slot"),
                    label: if source.display_name.trim().is_empty() {
                        "Media source".into()
                    } else {
                        source.display_name.chars().take(80).collect()
                    },
                    requires_video,
                    requires_audio,
                    still_image: source.still_image,
                    width: source.width,
                    height: source.height,
                    extra: BTreeMap::new(),
                };
                source_replacements.insert(source_id.to_owned(), slot.id.clone());
                source_slots.push(slot);
            }
        }

        let mut clips = project.clips.clone();
        let mut canvas_fill_clip_ids = Vec::new();
        for clip in &mut clips {
            if is_template_canvas_fill(clip, project) {
                canvas_fill_clip_ids.push(clip.id.clone());
            }
            if let Some(source_id) = &clip.source_id {
                clip.source_id = Some(
                    source_replacements
                        .get(source_id)
                        .with_context(|| {
                            format!("source {source_id:?} could not be made portable")
                        })?
                        .clone(),
                );
            }
            if let Some(caption) = &mut clip.caption {
                caption.source_id = None;
                caption.source_clip_id = None;
            }
        }

        let mut template = Self {
            id: unique_id("template"),
            name: name.into(),
            base_width: project.width,
            base_height: project.height,
            fps: project.fps,
            duration: project.duration,
            source_slots,
            generated_sources,
            canvas_fill_clip_ids,
            tracks: project.tracks.clone(),
            clips,
            transitions: project.transitions.clone(),
            markers: project.markers.clone(),
            ..Self::default()
        };
        template.text_parameters = template.discover_text_parameters()?;
        template.validate()?;
        Ok(template)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != COMPOSITION_TEMPLATE_FORMAT {
            bail!("not a Sono-Renderer presentation template");
        }
        if self.schema_version != COMPOSITION_TEMPLATE_SCHEMA_VERSION {
            bail!(
                "unsupported presentation-template schema version {}",
                self.schema_version
            );
        }
        if self.id.trim().is_empty() || self.id.len() > 128 {
            bail!("template identity must contain 1..=128 characters");
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 128 {
            bail!("template name must contain 1..=128 characters");
        }
        if self.base_width < 2
            || self.base_height < 2
            || self.base_width > 8192
            || self.base_height > 8192
            || self.base_width % 2 != 0
            || self.base_height % 2 != 0
        {
            bail!("template base dimensions must be even and in 2..=8192");
        }
        finite_range(self.fps as f64, 1.0, 240.0, "template frame rate")?;
        finite_range(
            self.duration,
            1.0 / f64::from(self.fps),
            86_400.0,
            "template duration",
        )?;
        if self.source_slots.len() > MAX_TEMPLATE_SOURCE_SLOTS
            || self.clips.len() > MAX_TEMPLATE_CLIPS
            || self.tracks.len() > 256
            || self.generated_sources.len() > MAX_TEMPLATE_SOURCE_SLOTS
        {
            bail!("presentation template exceeds a supported object limit");
        }

        let mut fill_clip_ids = HashSet::new();
        for clip_id in &self.canvas_fill_clip_ids {
            let clip = self
                .clips
                .iter()
                .find(|clip| clip.id == *clip_id)
                .context("template canvas-fill rule refers to a missing clip")?;
            if clip.kind != ClipKind::Video
                || clip.source_id.is_none()
                || !fill_clip_ids.insert(clip_id.as_str())
            {
                bail!("template canvas-fill rules must identify unique sourced video clips");
            }
        }

        let mut slot_ids = HashSet::new();
        for slot in &self.source_slots {
            if slot.id.trim().is_empty()
                || slot.id.len() > 128
                || !slot_ids.insert(slot.id.as_str())
                || slot.label.trim().is_empty()
                || slot.label.chars().count() > 80
                || (!slot.requires_video && !slot.requires_audio)
            {
                bail!("template contains an invalid or duplicate media source slot");
            }
        }

        let mut parameter_names = HashSet::new();
        for parameter in &self.text_parameters {
            if !valid_template_parameter(&parameter.name)
                || !parameter_names.insert(parameter.name.as_str())
                || parameter.default_value.chars().count() > 16_384
            {
                bail!("template contains an invalid, duplicate, or oversized text parameter");
            }
        }
        let discovered = self.discover_text_parameters()?;
        if discovered != self.text_parameters {
            bail!("template text parameter declarations do not match the tokens used by its text clips");
        }
        for clip in &self.clips {
            if clip.font_path.is_some() {
                bail!("presentation templates cannot contain machine-specific font paths");
            }
            if let Some(source_id) = &clip.source_id {
                let slot = slot_ids.contains(source_id.as_str());
                let generated = self
                    .generated_sources
                    .iter()
                    .any(|source| source.id == *source_id);
                if !slot && !generated {
                    bail!(
                        "template clip {:?} refers to an undeclared source slot",
                        clip.name
                    );
                }
            }
        }
        let mut generated_ids = HashSet::new();
        for source in &self.generated_sources {
            if source.id.trim().is_empty()
                || !generated_ids.insert(source.id.as_str())
                || source.solid_color.is_none()
                || source.nested_composition_path.is_some()
                || source.proxy_path.is_some()
                || source.proxy_metadata.is_some()
                || source.has_audio
            {
                bail!("templates may only include unique, path-free procedural visual sources");
            }
        }

        // Reuse the production composition validator for clip, track, effect,
        // mask, keyframe, group and transition invariants. Slot sources here
        // are path-free metadata stand-ins; concrete bindings are revalidated
        // against the actual media when the template is applied.
        let mut composition = CompositionProject::new();
        composition.name = self.name.clone();
        composition.width = self.base_width;
        composition.height = self.base_height;
        composition.fps = self.fps;
        composition.duration = self.duration;
        composition.tracks = self.tracks.clone();
        composition.clips = self.clips.clone();
        composition.transitions = self.transitions.clone();
        composition.markers = self.markers.clone();
        composition.sources = self.generated_sources.clone();
        composition
            .sources
            .extend(self.source_slots.iter().map(|slot| MediaSource {
                id: slot.id.clone(),
                display_name: slot.label.clone(),
                has_video: slot.requires_video,
                has_audio: slot.requires_audio,
                duration: if slot.still_image { 0.0 } else { 86_400.0 },
                width: slot.width.or(Some(self.base_width)),
                height: slot.height.or(Some(self.base_height)),
                still_image: slot.still_image,
                ..Default::default()
            }));
        composition
            .validate()
            .context("validating template objects")?;
        Ok(())
    }

    fn discover_text_parameters(&self) -> Result<Vec<TemplateTextParameter>> {
        let mut parameters = BTreeMap::<String, String>::new();
        for clip in self.clips.iter().filter(|clip| clip.kind == ClipKind::Text) {
            let mut rest = clip.text.as_str();
            while let Some(start) = rest.find("{{param:") {
                rest = &rest[start + 8..];
                let end = rest
                    .find("}}")
                    .context("template parameter token is missing its closing '}}'")?;
                let token = &rest[..end];
                let (name, default_value) = token
                    .split_once('|')
                    .map_or((token, ""), |(name, value)| (name, value));
                if !valid_template_parameter(name) || default_value.chars().count() > 16_384 {
                    bail!("invalid template text parameter token {{{{param:{token}}}}}");
                }
                if let Some(previous) = parameters.insert(name.into(), default_value.into()) {
                    if previous != default_value {
                        bail!("template parameter {name:?} has conflicting defaults");
                    }
                }
                rest = &rest[end + 2..];
            }
        }
        Ok(parameters
            .into_iter()
            .map(|(name, default_value)| TemplateTextParameter {
                name,
                default_value,
            })
            .collect())
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        self.validate()?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("creating template directory {}", parent.display()))?;
        let mut stored = self.clone();
        stored.schema_version = COMPOSITION_TEMPLATE_SCHEMA_VERSION;
        let bytes =
            serde_json::to_vec_pretty(&stored).context("serializing presentation template")?;
        if bytes.len() as u64 > COMPOSITION_TEMPLATE_FILE_LIMIT {
            bail!("presentation template exceeds the {COMPOSITION_TEMPLATE_FILE_LIMIT}-byte limit");
        }
        let mut staged = tempfile::NamedTempFile::new_in(parent)
            .context("creating presentation-template staging file")?;
        staged.write_all(&bytes)?;
        staged.as_file().sync_all()?;
        staged
            .persist(path)
            .map_err(|error| error.error)
            .with_context(|| {
                format!("atomically saving presentation template {}", path.display())
            })?;
        Ok(())
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)
            .with_context(|| format!("accessing presentation template {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > COMPOSITION_TEMPLATE_FILE_LIMIT {
            bail!("presentation template must be a regular file within the {COMPOSITION_TEMPLATE_FILE_LIMIT}-byte limit");
        }
        let bytes = fs::read(path)
            .with_context(|| format!("reading presentation template {}", path.display()))?;
        let template: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing presentation template {}", path.display()))?;
        template.validate()?;
        Ok(template)
    }

    /// Instantiate a template as ordinary editable NLE objects. Every source
    /// slot must be explicitly bound to an already imported media-bin source.
    pub fn apply_to_project(
        &self,
        project: &mut CompositionProject,
        bindings: &HashMap<String, String>,
        text_values: &HashMap<String, String>,
        responsive: bool,
    ) -> Result<Vec<String>> {
        self.validate()?;
        project.validate()?;
        if !responsive && (project.width != self.base_width || project.height != self.base_height) {
            bail!("template canvas differs from the composition; enable responsive layout or use a matching canvas");
        }
        if project.clips.len().saturating_add(self.clips.len()) > MAX_TEMPLATE_CLIPS {
            bail!("applying the template would exceed the supported composition clip limit");
        }

        let mut resolved_sources = HashMap::new();
        for slot in &self.source_slots {
            let source_id = bindings.get(&slot.id).with_context(|| {
                format!("choose a media source for template slot {:?}", slot.label)
            })?;
            let source = project
                .sources
                .iter()
                .find(|source| source.id == *source_id)
                .with_context(|| {
                    format!("media source for slot {:?} no longer exists", slot.label)
                })?;
            if (slot.requires_video && !source.has_video)
                || (slot.requires_audio && !source.has_audio)
                || (slot.still_image && !source.still_image)
            {
                bail!(
                    "media source {:?} does not satisfy template slot {:?}",
                    source.display_name,
                    slot.label
                );
            }
            resolved_sources.insert(slot.id.as_str(), source.clone());
        }
        for name in text_values.keys() {
            if !self
                .text_parameters
                .iter()
                .any(|parameter| parameter.name == *name)
            {
                bail!("template has no exposed text parameter {name:?}");
            }
        }

        let mut candidate = project.clone();
        candidate.duration = candidate.duration.max(self.duration);
        let mut track_ids = HashMap::new();
        let mut ordered_tracks = self.tracks.iter().collect::<Vec<_>>();
        ordered_tracks.sort_by_key(|track| (track.kind, track.order, track.id.as_str()));
        for track in ordered_tracks {
            let next_order = candidate
                .tracks
                .iter()
                .filter(|existing| existing.kind == track.kind)
                .map(|existing| existing.order)
                .max()
                .map_or(0, |order| order.saturating_add(1));
            let id = unique_id("track");
            let mut inserted = track.clone();
            inserted.id = id.clone();
            inserted.order = next_order;
            inserted.locked = false;
            track_ids.insert(track.id.as_str(), id);
            candidate.tracks.push(inserted);
        }

        let mut generated_source_ids = HashMap::new();
        for source in &self.generated_sources {
            let mut inserted = source.clone();
            let new_id = unique_id("solid");
            generated_source_ids.insert(source.id.as_str(), new_id.clone());
            inserted.id = new_id;
            candidate.sources.push(inserted);
        }
        let mut clip_ids = HashMap::new();
        let mut group_ids = HashMap::new();
        let mut link_ids = HashMap::new();
        let mut inserted_clips = Vec::with_capacity(self.clips.len());
        let z_offset = candidate
            .clips
            .iter()
            .map(|clip| clip.z_order)
            .max()
            .unwrap_or(-1)
            .saturating_add(1);
        let sx = f64::from(candidate.width) / f64::from(self.base_width);
        let sy = f64::from(candidate.height) / f64::from(self.base_height);
        let uniform = sx.min(sy);
        for template_clip in &self.clips {
            let mut clip = template_clip.clone();
            let new_id = unique_id("clip");
            clip_ids.insert(template_clip.id.as_str(), new_id.clone());
            clip.id = new_id;
            clip.track_id = template_clip
                .track_id
                .as_deref()
                .and_then(|id| track_ids.get(id).cloned());
            clip.z_order = z_offset.saturating_add(template_clip.z_order);
            if let Some(group_id) = &template_clip.group_id {
                let mapped = group_ids
                    .entry(group_id.clone())
                    .or_insert_with(|| unique_id("group"));
                clip.group_id = Some(mapped.clone());
            }
            if let Some(link_id) = &template_clip.link_group_id {
                let mapped = link_ids
                    .entry(link_id.clone())
                    .or_insert_with(|| unique_id("link"));
                clip.link_group_id = Some(mapped.clone());
            }
            if let Some(source_id) = &template_clip.source_id {
                if let Some(source_id) = generated_source_ids.get(source_id.as_str()) {
                    clip.source_id = Some(source_id.clone());
                } else if let Some(source) = resolved_sources.get(source_id.as_str()) {
                    clip.source_id = Some(source.id.clone());
                    if clip.kind == ClipKind::Video && !source.still_image {
                        if let (
                            Some(old_width),
                            Some(old_height),
                            Some(new_width),
                            Some(new_height),
                        ) = (
                            self.source_slots
                                .iter()
                                .find(|slot| slot.id == *source_id)
                                .and_then(|slot| slot.width),
                            self.source_slots
                                .iter()
                                .find(|slot| slot.id == *source_id)
                                .and_then(|slot| slot.height),
                            source.width,
                            source.height,
                        ) {
                            if new_width > 0 && new_height > 0 {
                                clip.scale_x *= f64::from(old_width) / f64::from(new_width);
                                clip.scale_y *= f64::from(old_height) / f64::from(new_height);
                            }
                        }
                    }
                } else {
                    bail!("template source {:?} was not bound", source_id);
                }
            }
            if responsive {
                clip.x *= sx;
                clip.y *= sy;
                for key in clip
                    .animated_properties
                    .get_mut(&AnimatedProperty::X)
                    .into_iter()
                    .flatten()
                {
                    key.value *= sx;
                }
                for key in clip
                    .animated_properties
                    .get_mut(&AnimatedProperty::Y)
                    .into_iter()
                    .flatten()
                {
                    key.value *= sy;
                }
                for key in &mut clip.position_keyframes {
                    key.x *= sx;
                }
                if matches!(clip.kind, ClipKind::Video | ClipKind::Text) {
                    clip.scale_x *= uniform;
                    clip.scale_y *= uniform;
                }
                if clip.kind == ClipKind::Text {
                    clip.font_size = (f64::from(clip.font_size) * uniform)
                        .round()
                        .clamp(1.0, 512.0) as u32;
                    clip.outline_width *= uniform;
                    clip.shadow_x *= uniform;
                    clip.shadow_y *= uniform;
                    clip.text_background_padding =
                        (f64::from(clip.text_background_padding) * uniform)
                            .round()
                            .clamp(0.0, 256.0) as u32;
                }
            }
            if responsive
                && self
                    .canvas_fill_clip_ids
                    .iter()
                    .any(|id| id == &template_clip.id)
            {
                if let Some(source) = clip.source_id.as_deref().and_then(|source_id| {
                    candidate
                        .sources
                        .iter()
                        .find(|source| source.id == source_id)
                }) {
                    if let (Some(width), Some(height)) = (source.width, source.height) {
                        if width > 0 && height > 0 {
                            let cover_scale = (f64::from(candidate.width) / f64::from(width))
                                .max(f64::from(candidate.height) / f64::from(height));
                            let scale_x_ratio = cover_scale / clip.scale_x;
                            let scale_y_ratio = cover_scale / clip.scale_y;
                            if let Some(keys) =
                                clip.animated_properties.get_mut(&AnimatedProperty::ScaleX)
                            {
                                for key in keys {
                                    key.value *= scale_x_ratio;
                                }
                            }
                            if let Some(keys) =
                                clip.animated_properties.get_mut(&AnimatedProperty::ScaleY)
                            {
                                for key in keys {
                                    key.value *= scale_y_ratio;
                                }
                            }
                            clip.scale_x = cover_scale;
                            clip.scale_y = cover_scale;
                            clip.x =
                                (f64::from(candidate.width) - f64::from(width) * cover_scale) / 2.0;
                            clip.y = (f64::from(candidate.height)
                                - f64::from(height) * cover_scale)
                                / 2.0;
                        }
                    }
                }
            }
            if clip.kind == ClipKind::Text {
                clip.text =
                    substitute_template_text(&clip.text, &self.text_parameters, text_values)?;
            }
            if let Some(caption) = &mut clip.caption {
                caption.source_id = None;
                caption.source_clip_id = None;
            }
            for effect in &mut clip.effects {
                effect.id = unique_id("effect");
            }
            inserted_clips.push(clip);
        }
        candidate.clips.extend(inserted_clips);

        for transition in &self.transitions {
            let mut inserted = transition.clone();
            inserted.id = unique_id("transition");
            inserted.from_clip_id = clip_ids
                .get(transition.from_clip_id.as_str())
                .context("template transition lost its outgoing clip")?
                .clone();
            inserted.to_clip_id = clip_ids
                .get(transition.to_clip_id.as_str())
                .context("template transition lost its incoming clip")?
                .clone();
            candidate.transitions.push(inserted);
        }
        for marker in &self.markers {
            let mut inserted = marker.clone();
            inserted.id = unique_id("marker");
            candidate.markers.push(inserted);
        }
        candidate
            .validate()
            .context("template could not be applied to the current composition")?;
        let ids = self
            .clips
            .iter()
            .filter_map(|clip| clip_ids.get(clip.id.as_str()).cloned())
            .collect();
        *project = candidate;
        Ok(ids)
    }
}

fn is_template_canvas_fill(clip: &CompositionClip, project: &CompositionProject) -> bool {
    if clip.kind != ClipKind::Video
        || clip.source_id.is_none()
        || clip.x.abs() > 0.5
        || clip.y.abs() > 0.5
        || clip.rotation_degrees.abs() > 0.001
        || clip.crop_left.abs() > 0.001
        || clip.crop_top.abs() > 0.001
        || clip.crop_right.abs() > 0.001
        || clip.crop_bottom.abs() > 0.001
        || clip.mask.is_some()
        || clip.animated_properties.contains_key(&AnimatedProperty::X)
        || clip.animated_properties.contains_key(&AnimatedProperty::Y)
        || !clip.position_keyframes.is_empty()
    {
        return false;
    }
    let Some(source) = clip
        .source_id
        .as_deref()
        .and_then(|id| project.sources.iter().find(|source| source.id == id))
    else {
        return false;
    };
    if source.generated_shape.is_some() {
        return false;
    }
    let (Some(width), Some(height)) = (source.width, source.height) else {
        return false;
    };
    if width == 0 || height == 0 || clip.scale_x <= 0.0 || clip.scale_y <= 0.0 {
        return false;
    }
    let contain = (f64::from(project.width) / f64::from(width))
        .min(f64::from(project.height) / f64::from(height));
    let tolerance = (contain * 0.01).max(0.01);
    (clip.scale_x - contain).abs() <= tolerance && (clip.scale_y - contain).abs() <= tolerance
}

fn valid_template_parameter(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn substitute_template_text(
    text: &str,
    parameters: &[TemplateTextParameter],
    values: &HashMap<String, String>,
) -> Result<String> {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("{{param:") {
        result.push_str(&rest[..start]);
        rest = &rest[start + 8..];
        let end = rest
            .find("}}")
            .context("template parameter token is missing its closing '}}'")?;
        let token = &rest[..end];
        let (name, _embedded_default) = token
            .split_once('|')
            .map_or((token, ""), |(name, default)| (name, default));
        let parameter = parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .with_context(|| format!("template text refers to undeclared parameter {name:?}"))?;
        let value = values.get(name).unwrap_or(&parameter.default_value);
        if value.chars().count() > 16_384 {
            bail!("template value {name:?} exceeds the supported text length");
        }
        result.push_str(value);
        rest = &rest[end + 2..];
    }
    result.push_str(rest);
    if result.chars().count() > 16_384 {
        bail!("resolved template text exceeds the supported text length");
    }
    Ok(result)
}

impl Default for CompositionRange {
    fn default() -> Self {
        Self {
            start: 0.0,
            end: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CompositionMarker {
    pub id: String,
    pub time: f64,
    pub label: String,
    pub color: String,
}

impl Default for CompositionMarker {
    fn default() -> Self {
        Self {
            id: String::new(),
            time: 0.0,
            label: String::new(),
            color: "FFD54F".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct CompositionRecoveryEnvelope {
    format: String,
    schema_version: u32,
    project_path: Option<PathBuf>,
    checkpoint_unix_ms: u128,
    project: CompositionProject,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CompositionRecoverySnapshot {
    pub project_path: Option<PathBuf>,
    pub checkpoint_unix_ms: u128,
    pub project: CompositionProject,
}

impl Default for CompositionProject {
    fn default() -> Self {
        Self::new()
    }
}

impl CompositionProject {
    pub fn new() -> Self {
        Self {
            format: COMPOSITION_FORMAT.into(),
            schema_version: COMPOSITION_SCHEMA_VERSION,
            composition_id: unique_id("composition"),
            name: "Untitled Composition".into(),
            width: 1080,
            height: 1920,
            fps: 30,
            duration: 30.0,
            export_range: None,
            export_preset: CompositionExportPreset::default(),
            use_proxies_for_preview: false,
            title_presets: default_title_presets(),
            sources: Vec::new(),
            tracks: vec![
                CompositionTrack::new("visual-main", "Visual 1", TrackKind::Visual, 0),
                CompositionTrack::new("audio-main", "Audio 1", TrackKind::Audio, 0),
            ],
            clips: Vec::new(),
            markers: Vec::new(),
            transitions: Vec::new(),
            extra: BTreeMap::new(),
        }
    }

    /// Build an ephemeral ordinary composition for auditioning one audio
    /// source. This uses the production media import, timeline mixer, and
    /// playback clock; it does not load a Sonolus engine or Watch runtime.
    pub fn single_audio_preview(path: impl AsRef<Path>) -> Result<Self> {
        let mut project = Self::new();
        let source_id = project.add_source(path)?;
        let source = project
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("imported audio source disappeared")?;
        if !source.has_audio || !source.duration.is_finite() || source.duration <= 0.0 {
            bail!("selected media does not contain a usable audio stream");
        }
        project.duration = source.duration;
        project.add_clip_to_track(&source_id, ClipKind::Audio, 0.0, Some("audio-main"))?;
        project.validate()?;
        Ok(project)
    }

    /// Build the same ordinary audio-only timeline using only already present
    /// FFmpeg tools. Chart Studio uses this for explicit audition so opening
    /// its music panel never triggers dependency provisioning.
    pub(crate) fn single_audio_preview_with_existing_tools(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .with_context(|| format!("opening media {}", path.display()))?;
        let tools = crate::ffmpeg::inspect_existing()?
            .context("audio audition needs the application's existing FFmpeg installation")?;
        let probe = probe_media_offline_with_tools(&path, &tools)?;
        if !probe.has_audio || !probe.duration.is_finite() || probe.duration <= 0.0 {
            bail!("selected media does not contain a usable audio stream");
        }
        let mut project = Self::new();
        let id = unique_id("source");
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Audio")
            .chars()
            .take(512)
            .collect();
        project.sources.push(MediaSource {
            id: id.clone(),
            path,
            display_name,
            has_video: false,
            has_audio: true,
            duration: probe.duration,
            waveform_peaks: Vec::new(),
            ..Default::default()
        });
        project.duration = probe.duration;
        project.add_clip_to_track(&id, ClipKind::Audio, 0.0, Some("audio-main"))?;
        project.validate()?;
        Ok(project)
    }

    pub fn set_export_range(&mut self, start: f64, end: f64) -> Result<()> {
        let mut candidate = self.clone();
        candidate.export_range = Some(CompositionRange { start, end });
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn clear_export_range(&mut self) {
        self.export_range = None;
    }

    pub fn export_duration(&self) -> f64 {
        self.export_range
            .map_or(self.duration, |range| range.end - range.start)
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != COMPOSITION_FORMAT {
            bail!("not a Sono-Renderer composition document");
        }
        if self.schema_version != COMPOSITION_SCHEMA_VERSION {
            bail!(
                "unsupported composition schema version {}",
                self.schema_version
            );
        }
        if self.composition_id.trim().is_empty() || self.composition_id.len() > 128 {
            bail!("composition identity must contain 1..=128 characters");
        }
        if self.width == 0 || self.height == 0 || self.width > 8192 || self.height > 8192 {
            bail!("composition dimensions must be in 1..=8192");
        }
        if self.fps == 0 || self.fps > 240 {
            bail!("composition frame rate must be in 1..=240");
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            bail!("MP4 composition dimensions must be even");
        }
        finite_range(
            self.duration,
            1.0 / f64::from(self.fps),
            86_400.0,
            "composition duration",
        )?;
        if let Some(range) = self.export_range {
            finite_range(range.start, 0.0, self.duration, "export range in-point")?;
            finite_range(range.end, 0.0, self.duration, "export range out-point")?;
            if range.end - range.start < 1.0 / f64::from(self.fps) {
                bail!("export range must contain at least one frame");
            }
        }
        let mut preset_ids = HashSet::new();
        let mut preset_names = HashSet::new();
        if self.title_presets.len() > 256 {
            bail!("a composition can contain at most 256 title presets");
        }
        for preset in &self.title_presets {
            if preset.id.trim().is_empty() || !preset_ids.insert(preset.id.as_str()) {
                bail!("title preset IDs must be nonempty and unique");
            }
            let normalized_name = preset.name.trim().to_lowercase();
            if normalized_name.is_empty()
                || preset.name.chars().count() > 64
                || !preset_names.insert(normalized_name)
            {
                bail!("title preset names must be unique and contain 1..=64 characters");
            }
            if preset.sample_text.chars().count() > 16_384 {
                bail!("title preset sample text exceeds the supported limit");
            }
            finite_range(preset.x, 0.0, 1.0, "title preset horizontal position")?;
            finite_range(preset.y, 0.0, 1.0, "title preset vertical position")?;
            if preset.font_size == 0 || preset.font_size > 512 {
                bail!("title preset font size must be in 1..=512");
            }
            validate_color(&preset.text_color)?;
            validate_color(&preset.outline_color)?;
            validate_color(&preset.text_background_color)?;
            finite_range(
                preset.outline_width,
                0.0,
                32.0,
                "title preset outline width",
            )?;
            finite_range(preset.shadow_x, -128.0, 128.0, "title preset shadow x")?;
            finite_range(preset.shadow_y, -128.0, 128.0, "title preset shadow y")?;
            finite_range(
                preset.text_background_opacity,
                0.0,
                1.0,
                "title preset background opacity",
            )?;
            if preset.text_background_padding > 256 {
                bail!("title preset background padding cannot exceed 256 pixels");
            }
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 200 {
            bail!("composition name must contain 1..=200 characters");
        }
        let mut track_ids = HashSet::new();
        for track in &self.tracks {
            if track.id.trim().is_empty() || !track_ids.insert(track.id.as_str()) {
                bail!("track IDs must be nonempty and unique");
            }
            if track.name.trim().is_empty() || track.name.chars().count() > 200 {
                bail!("track names must contain 1..=200 characters");
            }
            finite_range(track.gain, 0.0, 16.0, "audio track gain")?;
        }
        if !self
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Visual)
            || !self
                .tracks
                .iter()
                .any(|track| track.kind == TrackKind::Audio)
        {
            bail!("a composition must contain at least one visual and one audio track");
        }
        let mut source_ids = HashSet::new();
        for source in &self.sources {
            if source.id.trim().is_empty() || !source_ids.insert(source.id.as_str()) {
                bail!("media source IDs must be nonempty and unique");
            }
            let generated_visual = source.still_image || source.solid_color.is_some();
            if !source.duration.is_finite()
                || (generated_visual && source.duration < 0.0)
                || (!generated_visual && source.duration <= 0.0)
            {
                bail!("source {:?} has an invalid duration", source.display_name);
            }
            if source.solid_color.is_some() {
                if source.still_image || !source.has_video || source.has_audio {
                    bail!("generated solid sources must be visual-only");
                }
                validate_color(source.solid_color.as_deref().unwrap())?;
            }
            if let Some(shape) = source.generated_shape {
                if source.solid_color.is_none() {
                    bail!("generated shapes must have a solid fill color");
                }
                if !shape.x.is_finite()
                    || !shape.y.is_finite()
                    || !shape.width.is_finite()
                    || !shape.height.is_finite()
                    || shape.x < 0.0
                    || shape.y < 0.0
                    || shape.width <= 0.0
                    || shape.height <= 0.0
                    || shape.x + shape.width > 1.0
                    || shape.y + shape.height > 1.0
                {
                    bail!("generated shape bounds must fit within the source canvas");
                }
            }
            if source.nested_composition_path.is_some()
                && (source.still_image || source.solid_color.is_some() || !source.has_video)
            {
                bail!("nested composition sources must be visual compositions");
            }
            if source.proxy_path.is_some()
                && (!source.has_video
                    || source.still_image
                    || source.solid_color.is_some()
                    || source.nested_composition_path.is_some())
            {
                bail!("only ordinary video media can have a video proxy");
            }
            if let Some(proxy) = &source.proxy_metadata {
                if source.proxy_path.is_none() || proxy.source_id != source.id {
                    bail!("video proxy metadata must belong to its canonical source");
                }
                if proxy.width < 2
                    || proxy.height < 2
                    || !proxy.duration.is_finite()
                    || proxy.duration <= 0.0
                    || proxy.video_codec.is_empty()
                    || proxy.video_codec.len() > 80
                {
                    bail!("video proxy metadata is invalid");
                }
            }
            if source.still_image && (!source.has_video || source.has_audio) {
                bail!("still image sources must contain only a video stream");
            }
            if source.display_name.chars().count() > 512 {
                bail!("media source display name is too long");
            }
            if source.waveform_peaks.len() > 16_384
                || source
                    .waveform_peaks
                    .iter()
                    .any(|peak| !peak.is_finite() || !(0.0..=1.0).contains(peak))
            {
                bail!("media source waveform summary is invalid or too large");
            }
        }
        let sources: HashMap<_, _> = self
            .sources
            .iter()
            .map(|source| (source.id.as_str(), source))
            .collect();
        let mut clip_ids = HashSet::new();
        let mut effect_ids = HashSet::new();
        let mut linked_clips = HashMap::<&str, Vec<&CompositionClip>>::new();
        let mut grouped_clips = HashMap::<&str, Vec<&CompositionClip>>::new();
        for clip in &self.clips {
            if clip.id.trim().is_empty() || !clip_ids.insert(clip.id.as_str()) {
                bail!("clip IDs must be nonempty and unique");
            }
            finite_range(
                clip.timeline_start,
                0.0,
                self.duration,
                "clip timeline start",
            )?;
            finite_range(clip.source_start, 0.0, 86_400.0, "clip source in-point")?;
            finite_range(clip.playback_rate, 0.1, 8.0, "clip playback rate")?;
            finite_range(
                clip.duration,
                1.0 / f64::from(self.fps),
                86_400.0,
                "clip duration",
            )?;
            finite_range(clip.x, -32_768.0, 32_768.0, "clip x")?;
            finite_range(clip.y, -32_768.0, 32_768.0, "clip y")?;
            finite_range(clip.scale_x, 0.01, 16.0, "clip horizontal scale")?;
            finite_range(clip.scale_y, 0.01, 16.0, "clip vertical scale")?;
            for (value, label) in [
                (clip.crop_left, "left crop"),
                (clip.crop_top, "top crop"),
                (clip.crop_right, "right crop"),
                (clip.crop_bottom, "bottom crop"),
            ] {
                finite_range(value, 0.0, 0.95, label)?;
            }
            if clip.crop_left + clip.crop_right >= 0.99 || clip.crop_top + clip.crop_bottom >= 0.99
            {
                bail!("clip crop must leave a visible source area");
            }
            finite_range(clip.opacity, 0.0, 1.0, "clip opacity")?;
            finite_range(clip.blur, 0.0, 100.0, "Gaussian blur sigma")?;
            if clip.effects.len() > 64 {
                bail!("a clip cannot contain more than 64 visual effects");
            }
            if !matches!(clip.kind, ClipKind::Video | ClipKind::Adjustment)
                && !clip.effects.is_empty()
            {
                bail!("visual effects can only be attached to video or adjustment clips");
            }
            for effect in &clip.effects {
                if effect.id.trim().is_empty() || !effect_ids.insert(effect.id.as_str()) {
                    bail!("visual effect IDs must be nonempty and unique in the composition");
                }
                validate_visual_effect(&effect.kind)?;
                validate_effect_parameter_keyframes(effect, self.duration)?;
                if clip.kind == ClipKind::Adjustment
                    && matches!(effect.kind, VisualEffectKind::ChromaKey { .. })
                {
                    bail!("chroma key is not supported on adjustment layers");
                }
            }
            if let Some(mask) = &clip.mask {
                if clip.kind != ClipKind::Video {
                    bail!("visual masks can only be attached to video clips");
                }
                finite_range(mask.x, 0.0, 1.0, "mask x")?;
                finite_range(mask.y, 0.0, 1.0, "mask y")?;
                finite_range(mask.width, 0.01, 1.0, "mask width")?;
                finite_range(mask.height, 0.01, 1.0, "mask height")?;
                finite_range(mask.feather, 0.0, 0.5, "mask feather")?;
                if mask.x + mask.width > 1.0 || mask.y + mask.height > 1.0 {
                    bail!("visual mask bounds must remain inside the clip area");
                }
            }
            if clip.kind != ClipKind::Video
                && [
                    AnimatedProperty::MaskX,
                    AnimatedProperty::MaskY,
                    AnimatedProperty::MaskWidth,
                    AnimatedProperty::MaskHeight,
                    AnimatedProperty::MaskFeather,
                ]
                .iter()
                .any(|property| clip.animated_properties.contains_key(property))
            {
                bail!("mask animation can only be attached to video clips");
            }
            if clip.mask.is_none()
                && [
                    AnimatedProperty::MaskX,
                    AnimatedProperty::MaskY,
                    AnimatedProperty::MaskWidth,
                    AnimatedProperty::MaskHeight,
                    AnimatedProperty::MaskFeather,
                ]
                .iter()
                .any(|property| clip.animated_properties.contains_key(property))
            {
                bail!("mask keyframes require a visual mask");
            }
            finite_range(clip.gain, 0.0, 16.0, "audio gain")?;
            finite_range(clip.pan, -1.0, 1.0, "audio pan")?;
            finite_range(clip.fade_in, 0.0, clip.duration, "audio fade-in")?;
            finite_range(clip.fade_out, 0.0, clip.duration, "audio fade-out")?;
            if clip.fade_in + clip.fade_out > clip.duration {
                bail!("audio fade durations exceed the clip duration");
            }
            if clip.font_size == 0 || clip.font_size > 512 || clip.text.chars().count() > 16_384 {
                bail!("text layer font size or text length is outside supported limits");
            }
            validate_color(&clip.text_color)?;
            validate_color(&clip.outline_color)?;
            validate_color(&clip.text_background_color)?;
            finite_range(
                clip.text_background_opacity,
                0.0,
                1.0,
                "text background opacity",
            )?;
            if clip.text_background_padding > 256 {
                bail!("text background padding cannot exceed 256 pixels");
            }
            finite_range(clip.outline_width, 0.0, 32.0, "text outline width")?;
            finite_range(clip.shadow_x, -128.0, 128.0, "text shadow x")?;
            finite_range(clip.shadow_y, -128.0, 128.0, "text shadow y")?;
            if clip.timeline_start + clip.duration > self.duration + 1.0 / f64::from(self.fps) {
                bail!(
                    "clip {:?} extends beyond the composition duration",
                    clip.name
                );
            }
            if let Some(link_id) = clip.link_group_id.as_deref() {
                if link_id.trim().is_empty() {
                    bail!("linked clip group IDs must not be empty");
                }
                linked_clips.entry(link_id).or_default().push(clip);
            }
            if let Some(group_id) = clip.group_id.as_deref() {
                if group_id.trim().is_empty() {
                    bail!("user group IDs must not be empty");
                }
                grouped_clips.entry(group_id).or_default().push(clip);
            }
            match clip.kind {
                ClipKind::Text => {
                    if clip.source_id.is_some() || clip.text.trim().is_empty() {
                        bail!("text clips require text and must not reference a media source");
                    }
                }
                ClipKind::Adjustment => {
                    if clip.source_id.is_some() {
                        bail!(
                            "adjustment layers operate on the composite and cannot reference media"
                        );
                    }
                    if clip.source_start != 0.0
                        || clip.playback_rate != 1.0
                        || clip.source_time_mode != SourceTimeMode::Forward
                    {
                        bail!("adjustment layers do not have source-time mapping");
                    }
                    if clip.include_audio
                        || clip.has_animation(AnimatedProperty::Gain)
                        || clip.has_animation(AnimatedProperty::Pan)
                    {
                        bail!("adjustment layers cannot route or animate audio");
                    }
                    if clip.has_animation(AnimatedProperty::X)
                        || clip.has_animation(AnimatedProperty::Y)
                        || clip.has_animation(AnimatedProperty::ScaleX)
                        || clip.has_animation(AnimatedProperty::ScaleY)
                        || clip.has_animation(AnimatedProperty::Rotation)
                        || clip.has_animation(AnimatedProperty::CropLeft)
                        || clip.has_animation(AnimatedProperty::CropTop)
                        || clip.has_animation(AnimatedProperty::CropRight)
                        || clip.has_animation(AnimatedProperty::CropBottom)
                        || clip.has_animation(AnimatedProperty::Blur)
                        || !clip.position_keyframes.is_empty()
                    {
                        bail!("adjustment layers only animate opacity in the current compositor");
                    }
                    if clip.x != 0.0
                        || clip.y != 0.0
                        || clip.scale_x != 1.0
                        || clip.scale_y != 1.0
                        || clip.rotation_degrees != 0.0
                        || clip.crop_left != 0.0
                        || clip.crop_top != 0.0
                        || clip.crop_right != 0.0
                        || clip.crop_bottom != 0.0
                        || clip.blur != 0.0
                    {
                        bail!("adjustment layers use full-frame effects and do not have clip transforms");
                    }
                }
                ClipKind::Video | ClipKind::Audio => {
                    let source = clip
                        .source_id
                        .as_deref()
                        .and_then(|id| sources.get(id))
                        .with_context(|| {
                            format!("clip {:?} references a missing source", clip.name)
                        })?;
                    if clip.kind == ClipKind::Video && !source.has_video {
                        bail!(
                            "video clip {:?} references a source without video",
                            clip.name
                        );
                    }
                    if clip.kind == ClipKind::Audio && !source.has_audio {
                        bail!(
                            "audio clip {:?} references a source without audio",
                            clip.name
                        );
                    }
                    if source.still_image || source.solid_color.is_some() {
                        if clip.source_start != 0.0 {
                            bail!("still and generated visual clips have source in-point 0");
                        }
                    } else if clip.source_time_mode == SourceTimeMode::Freeze {
                        if clip.source_start >= source.duration {
                            bail!("freeze frame source time must be inside the media source");
                        }
                    } else {
                        finite_range(
                            clip.source_start + clip.duration * clip.playback_rate,
                            0.0,
                            source.duration + 1.0 / f64::from(self.fps),
                            "clip source end",
                        )?;
                    }
                    if clip.include_audio && !source.has_audio {
                        bail!(
                            "video clip {:?} enables audio for a source without an audio stream",
                            clip.name
                        );
                    }
                }
            }
            let mut last_time = -1.0;
            for key in &clip.position_keyframes {
                finite_range(key.time, 0.0, self.duration, "keyframe time")?;
                finite_range(key.x, -32_768.0, 32_768.0, "keyframe x")?;
                if key.time <= last_time {
                    bail!("position keyframes must have strictly increasing times");
                }
                last_time = key.time;
            }
            finite_range(clip.rotation_degrees, -36_000.0, 36_000.0, "clip rotation")?;
            if let Some(track_id) = &clip.track_id {
                let track = self
                    .tracks
                    .iter()
                    .find(|track| &track.id == track_id)
                    .with_context(|| format!("clip {:?} references a missing track", clip.name))?;
                let expected = match clip.kind {
                    ClipKind::Audio => TrackKind::Audio,
                    ClipKind::Video | ClipKind::Text | ClipKind::Adjustment => TrackKind::Visual,
                };
                if track.kind != expected {
                    bail!("clip {:?} is placed on an incompatible track", clip.name);
                }
            }
            for (property, keyframes) in &clip.animated_properties {
                let (min, max) = animated_property_range(*property);
                let mut last_time = -1.0;
                for key in keyframes {
                    finite_range(key.time, 0.0, self.duration, "keyframe time")?;
                    finite_range(key.value, min, max, "keyframe value")?;
                    if key.time <= last_time {
                        bail!(
                            "keyframes for {} must have strictly increasing times",
                            property.label()
                        );
                    }
                    last_time = key.time;
                }
            }
        }
        for (group_id, clips) in linked_clips {
            if clips.len() != 2
                || clips
                    .iter()
                    .filter(|clip| clip.kind == ClipKind::Video)
                    .count()
                    != 1
                || clips
                    .iter()
                    .filter(|clip| clip.kind == ClipKind::Audio)
                    .count()
                    != 1
            {
                bail!("linked A/V group {group_id:?} must contain exactly one video and one audio clip");
            }
            let [video, audio] = [clips[0], clips[1]];
            if video.source_id != audio.source_id
                || (video.timeline_start - audio.timeline_start).abs() > 0.000_001
                || (video.source_start - audio.source_start).abs() > 0.000_001
                || (video.duration - audio.duration).abs() > 0.000_001
                || (video.playback_rate - audio.playback_rate).abs() > 0.000_001
                || video.source_time_mode != audio.source_time_mode
            {
                bail!(
                    "linked A/V clips must reference the same source window and timeline interval"
                );
            }
        }
        for (group_id, clips) in grouped_clips {
            if clips.len() < 2 {
                bail!("user group {group_id:?} must contain at least two clips");
            }
        }
        let mut marker_ids = HashSet::new();
        for marker in &self.markers {
            if marker.id.trim().is_empty() || !marker_ids.insert(marker.id.as_str()) {
                bail!("timeline marker IDs must be nonempty and unique");
            }
            finite_range(marker.time, 0.0, self.duration, "timeline marker time")?;
            if marker.label.chars().count() > 200 {
                bail!("timeline marker label is too long");
            }
            validate_color(&marker.color)?;
        }
        let clips_by_id = self
            .clips
            .iter()
            .map(|clip| (clip.id.as_str(), clip))
            .collect::<HashMap<_, _>>();
        let mut transition_ids = HashSet::new();
        for transition in &self.transitions {
            if transition.id.trim().is_empty() || !transition_ids.insert(transition.id.as_str()) {
                bail!("transition IDs must be nonempty and unique");
            }
            if transition.from_clip_id == transition.to_clip_id {
                bail!("a transition must connect two different clips");
            }
            let from = clips_by_id
                .get(transition.from_clip_id.as_str())
                .context("transition references a missing outgoing clip")?;
            let to = clips_by_id
                .get(transition.to_clip_id.as_str())
                .context("transition references a missing incoming clip")?;
            if from.kind != ClipKind::Video
                || to.kind != ClipKind::Video
                || from.track_id != to.track_id
            {
                bail!("transitions require two video clips on the same track");
            }
            if to.z_order < from.z_order {
                bail!("the incoming transition clip must be above the outgoing clip");
            }
            finite_range(
                transition.start_time,
                0.0,
                self.duration,
                "transition start time",
            )?;
            finite_range(
                transition.duration,
                1.0 / f64::from(self.fps),
                self.duration,
                "transition duration",
            )?;
            let transition_end = transition.start_time + transition.duration;
            let shared_start = from.timeline_start.max(to.timeline_start);
            let shared_end =
                (from.timeline_start + from.duration).min(to.timeline_start + to.duration);
            if transition.start_time + 0.000_001 < shared_start
                || transition_end > shared_end + 0.000_001
            {
                bail!("transition placement must fit inside the clips' shared overlap");
            }
        }
        Ok(())
    }

    /// Resolve one animated scalar at composition time. Values hold before
    /// the first keyframe and after the last; duplicate times are rejected by
    /// validation instead of depending on ordering accidents.
    pub fn animated_value(
        &self,
        clip: &CompositionClip,
        property: AnimatedProperty,
        time: f64,
    ) -> f64 {
        clip.animated_value(property, time)
    }

    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> Result<String> {
        let name = name.into();
        if name.trim().is_empty() || name.chars().count() > 200 {
            bail!("track name must contain 1..=200 characters");
        }
        let order = self
            .tracks
            .iter()
            .filter(|track| track.kind == kind)
            .map(|track| track.order)
            .max()
            .map_or(0, |order| order.saturating_add(1));
        let id = unique_id("track");
        self.tracks
            .push(CompositionTrack::new(&id, &name, kind, order));
        Ok(id)
    }

    pub fn track_for_clip(&self, clip: &CompositionClip) -> Option<&CompositionTrack> {
        let expected = match clip.kind {
            ClipKind::Audio => TrackKind::Audio,
            ClipKind::Video | ClipKind::Text | ClipKind::Adjustment => TrackKind::Visual,
        };
        clip.track_id
            .as_deref()
            .and_then(|id| self.tracks.iter().find(|track| track.id == id))
            .or_else(|| {
                self.tracks
                    .iter()
                    .filter(|track| track.kind == expected)
                    .min_by_key(|track| (track.order, track.id.as_str()))
            })
    }

    pub fn add_transition_between(
        &mut self,
        from_clip_id: &str,
        to_clip_id: &str,
        kind: TransitionKind,
        duration: f64,
    ) -> Result<String> {
        finite_range(
            duration,
            1.0 / f64::from(self.fps),
            self.duration,
            "transition duration",
        )?;
        let from = self
            .clips
            .iter()
            .find(|clip| clip.id == from_clip_id)
            .context("outgoing transition clip no longer exists")?;
        let to = self
            .clips
            .iter()
            .find(|clip| clip.id == to_clip_id)
            .context("incoming transition clip no longer exists")?;
        let shared_start = from.timeline_start.max(to.timeline_start);
        let shared_end = (from.timeline_start + from.duration).min(to.timeline_start + to.duration);
        if shared_end - shared_start < duration {
            bail!("clips need at least {duration:.3}s of overlap for this transition");
        }
        let transition = CompositionTransition {
            id: unique_id("transition"),
            from_clip_id: from_clip_id.to_owned(),
            to_clip_id: to_clip_id.to_owned(),
            kind,
            easing: Easing::Linear,
            start_time: shared_start + (shared_end - shared_start - duration) / 2.0,
            duration,
        };
        let id = transition.id.clone();
        let mut candidate = self.clone();
        candidate.transitions.push(transition);
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    pub fn update_transition(
        &mut self,
        transition_id: &str,
        kind: TransitionKind,
        duration: f64,
    ) -> Result<()> {
        finite_range(
            duration,
            1.0 / f64::from(self.fps),
            self.duration,
            "transition duration",
        )?;
        let mut candidate = self.clone();
        let transition = candidate
            .transitions
            .iter_mut()
            .find(|transition| transition.id == transition_id)
            .context("selected transition no longer exists")?;
        let center = transition.start_time + transition.duration / 2.0;
        transition.kind = kind;
        transition.duration = duration;
        transition.start_time = center - duration / 2.0;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn set_transition_easing(&mut self, transition_id: &str, easing: Easing) -> Result<()> {
        let mut candidate = self.clone();
        let transition = candidate
            .transitions
            .iter_mut()
            .find(|transition| transition.id == transition_id)
            .context("selected transition no longer exists")?;
        transition.easing = easing;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn remove_transition(&mut self, transition_id: &str) -> Result<()> {
        let before = self.transitions.len();
        self.transitions
            .retain(|transition| transition.id != transition_id);
        if self.transitions.len() == before {
            bail!("selected transition no longer exists");
        }
        Ok(())
    }

    /// Apply a linear crossfade to an overlapping pair. The incoming clip
    /// begins during the outgoing clip and remains active through its end, so
    /// the two existing fade envelopes cover the same timeline interval.
    pub fn add_audio_crossfade(&mut self, first_id: &str, second_id: &str) -> Result<f64> {
        if first_id == second_id {
            bail!("an audio clip cannot be crossfaded with itself");
        }
        let mut candidate = self.clone();
        let first_index = candidate
            .clips
            .iter()
            .position(|clip| clip.id == first_id)
            .context("outgoing audio clip no longer exists")?;
        let second_index = candidate
            .clips
            .iter()
            .position(|clip| clip.id == second_id)
            .context("incoming audio clip no longer exists")?;
        let first = &candidate.clips[first_index];
        let second = &candidate.clips[second_index];
        if first.kind != ClipKind::Audio || second.kind != ClipKind::Audio {
            bail!("audio crossfades require two audio clips");
        }
        let (outgoing_index, incoming_index) = if first.timeline_start <= second.timeline_start {
            (first_index, second_index)
        } else {
            (second_index, first_index)
        };
        let outgoing = &candidate.clips[outgoing_index];
        let incoming = &candidate.clips[incoming_index];
        if outgoing.track_id != incoming.track_id {
            bail!("audio crossfade clips must be on the same audio track");
        }
        let outgoing_end = outgoing.timeline_start + outgoing.duration;
        let incoming_end = incoming.timeline_start + incoming.duration;
        let overlap = outgoing_end - incoming.timeline_start;
        let frame = 1.0 / f64::from(candidate.fps);
        if overlap < frame || incoming_end + frame * 0.5 < outgoing_end {
            bail!("audio clips must overlap with the incoming clip extending through the outgoing clip's end");
        }
        if candidate
            .track_for_clip(outgoing)
            .is_some_and(|track| track.locked)
            || candidate
                .track_for_clip(incoming)
                .is_some_and(|track| track.locked)
        {
            bail!("a locked audio track prevents adding a crossfade");
        }
        candidate.clips[outgoing_index].fade_out = overlap;
        candidate.clips[incoming_index].fade_in = overlap;
        candidate.validate()?;
        *self = candidate;
        Ok(overlap)
    }

    pub fn set_clip_source_time(
        &mut self,
        clip_id: &str,
        mode: SourceTimeMode,
        playback_rate: f64,
    ) -> Result<()> {
        finite_range(playback_rate, 0.1, 8.0, "clip playback rate")?;
        let selected = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        if matches!(selected.kind, ClipKind::Text | ClipKind::Adjustment) {
            bail!("text and adjustment clips do not have source-time mapping");
        }
        let link_id = selected.link_group_id.clone();
        let ids = self
            .clips
            .iter()
            .filter(|clip| {
                clip.id == clip_id
                    || link_id
                        .as_deref()
                        .is_some_and(|id| clip.link_group_id.as_deref() == Some(id))
            })
            .map(|clip| clip.id.clone())
            .collect::<Vec<_>>();
        if self
            .clips
            .iter()
            .filter(|clip| ids.contains(&clip.id))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents changing source-time mapping");
        }
        let mut candidate = self.clone();
        for clip in candidate
            .clips
            .iter_mut()
            .filter(|clip| ids.contains(&clip.id))
        {
            clip.source_time_mode = mode;
            clip.playback_rate = playback_rate;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn move_track(&mut self, track_id: &str, direction: isize) -> Result<()> {
        if direction == 0 {
            return Ok(());
        }
        let track = self
            .tracks
            .iter()
            .find(|track| track.id == track_id)
            .context("selected track no longer exists")?;
        let kind = track.kind;
        let mut ordered = self
            .tracks
            .iter()
            .filter(|track| track.kind == kind)
            .map(|track| track.id.clone())
            .collect::<Vec<_>>();
        ordered.sort_by_key(|id| {
            self.tracks
                .iter()
                .find(|track| track.id == *id)
                .map(|track| (track.order, track.id.clone()))
                .unwrap_or_default()
        });
        let current = ordered
            .iter()
            .position(|id| id == track_id)
            .context("selected track order is inconsistent")?;
        let target = (current as isize + direction).clamp(0, ordered.len() as isize - 1) as usize;
        if current != target {
            let id = ordered.remove(current);
            ordered.insert(target, id);
        }
        for (order, id) in ordered.iter().enumerate() {
            if let Some(track) = self.tracks.iter_mut().find(|track| track.id == *id) {
                track.order = order as i32;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn first_track_id(&self, kind: TrackKind) -> Option<String> {
        self.tracks
            .iter()
            .filter(|track| track.kind == kind)
            .min_by_key(|track| (track.order, track.id.as_str()))
            .map(|track| track.id.clone())
    }

    pub fn remove_track(&mut self, track_id: &str) -> Result<()> {
        let track = self
            .tracks
            .iter()
            .find(|track| track.id == track_id)
            .context("selected track no longer exists")?;
        if self
            .clips
            .iter()
            .any(|clip| clip.track_id.as_deref() == Some(track_id))
        {
            bail!("move or delete the track's clips before removing it");
        }
        if self
            .tracks
            .iter()
            .filter(|other| other.kind == track.kind)
            .count()
            <= 1
        {
            bail!("a composition must keep at least one track of each kind");
        }
        self.tracks.retain(|other| other.id != track_id);
        Ok(())
    }

    pub fn normalize_tracks(&mut self) {
        if !self
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Visual)
        {
            self.tracks.push(CompositionTrack::new(
                "visual-main",
                "Visual 1",
                TrackKind::Visual,
                0,
            ));
        }
        if !self
            .tracks
            .iter()
            .any(|track| track.kind == TrackKind::Audio)
        {
            self.tracks.push(CompositionTrack::new(
                "audio-main",
                "Audio 1",
                TrackKind::Audio,
                0,
            ));
        }
        for clip in &mut self.clips {
            if clip.track_id.is_none() {
                let kind = match clip.kind {
                    ClipKind::Audio => TrackKind::Audio,
                    ClipKind::Video | ClipKind::Text | ClipKind::Adjustment => TrackKind::Visual,
                };
                clip.track_id = self
                    .tracks
                    .iter()
                    .filter(|track| track.kind == kind)
                    .min_by_key(|track| (track.order, track.id.as_str()))
                    .map(|track| track.id.clone());
            }
            if !clip.position_keyframes.is_empty()
                && !clip.animated_properties.contains_key(&AnimatedProperty::X)
            {
                clip.animated_properties.insert(
                    AnimatedProperty::X,
                    clip.position_keyframes
                        .iter()
                        .map(|key| ScalarKeyframe {
                            time: key.time,
                            value: key.x,
                            easing_to_next: key.easing_to_next,
                        })
                        .collect(),
                );
            }
        }
    }

    pub fn add_source(&mut self, path: impl AsRef<Path>) -> Result<String> {
        let path = path
            .as_ref()
            .canonicalize()
            .with_context(|| format!("opening media {}", path.as_ref().display()))?;
        let probe = probe_media(&path)?;
        let id = unique_id("source");
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Media")
            .chars()
            .take(512)
            .collect();
        let waveform_peaks = if probe.has_audio {
            crate::media_audio::waveform_for_media(&path, probe.duration, 512).unwrap_or_default()
        } else {
            Vec::new()
        };
        let source = MediaSource {
            id: id.clone(),
            path,
            display_name,
            has_video: probe.has_video,
            has_audio: probe.has_audio,
            duration: if probe.still_image {
                0.0
            } else {
                probe.duration
            },
            width: probe.width,
            height: probe.height,
            still_image: probe.still_image,
            waveform_peaks,
            ..Default::default()
        };
        self.sources.push(source);
        Ok(id)
    }

    /// Add a live composition reference to the media bin. The child remains a
    /// `.sonocomp` document; render intermediates are created only in the
    /// caller's temporary workspace.
    pub fn add_nested_composition_source(&mut self, path: impl AsRef<Path>) -> Result<String> {
        let path = fs::canonicalize(path.as_ref())
            .with_context(|| format!("resolving nested composition {}", path.as_ref().display()))?;
        let child = CompositionProject::load(&path)?;
        let mut ancestors = vec![self.composition_id.clone()];
        validate_nested_graph(&child, &mut ancestors, &mut HashSet::new())?;
        let source = MediaSource {
            id: unique_id("nested-source"),
            nested_composition_path: Some(path),
            display_name: child.name.clone(),
            has_video: true,
            has_audio: child.routes_audio(),
            duration: child.duration,
            width: Some(child.width),
            height: Some(child.height),
            ..Default::default()
        };
        let id = source.id.clone();
        self.sources.push(source);
        if let Err(error) = self.validate() {
            self.sources.retain(|source| source.id != id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn relink_nested_composition_source(
        &mut self,
        source_id: &str,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        let path = fs::canonicalize(path.as_ref()).with_context(|| {
            format!(
                "resolving replacement nested composition {}",
                path.as_ref().display()
            )
        })?;
        let child = CompositionProject::load(&path)?;
        let mut ancestors = vec![self.composition_id.clone()];
        validate_nested_graph(&child, &mut ancestors, &mut HashSet::new())?;
        let index = self
            .sources
            .iter()
            .position(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        if self.sources[index].nested_composition_path.is_none() {
            bail!("selected media source is not a nested composition");
        }
        let mut candidate = self.clone();
        let source = &mut candidate.sources[index];
        source.path.clear();
        source.nested_composition_path = Some(path);
        source.display_name = child.name.clone();
        source.has_video = true;
        source.has_audio = child.routes_audio();
        source.duration = child.duration;
        source.width = Some(child.width);
        source.height = Some(child.height);
        source.waveform_peaks.clear();
        candidate.validate().context(
            "replacement composition is shorter than an existing clip or otherwise incompatible",
        )?;
        *self = candidate;
        Ok(())
    }

    fn routes_audio(&self) -> bool {
        self.clips.iter().any(|clip| {
            if !clip.enabled
                || clip.muted
                || !matches!(clip.kind, ClipKind::Audio | ClipKind::Video)
            {
                return false;
            }
            let routed = clip.kind == ClipKind::Audio || clip.include_audio;
            if !routed
                || self
                    .track_for_clip(clip)
                    .is_some_and(|track| !track.enabled || track.muted)
            {
                return false;
            }
            clip.source_id
                .as_deref()
                .and_then(|id| self.sources.iter().find(|source| source.id == id))
                .is_some_and(|source| source.has_audio)
        })
    }

    /// Remove project-bin entries that are not referenced by any timeline
    /// clip. This only edits composition metadata; it never deletes source
    /// files from disk.
    pub fn remove_unused_sources(&mut self) -> usize {
        let used: HashSet<&str> = self
            .clips
            .iter()
            .filter_map(|clip| clip.source_id.as_deref())
            .collect();
        let previous_len = self.sources.len();
        self.sources
            .retain(|source| used.contains(source.id.as_str()));
        previous_len - self.sources.len()
    }

    /// Replace the path and probed media facts for one canonical source. All
    /// clips keep the same source ID; a candidate that cannot satisfy their
    /// stream or source-time requirements is rejected without changing the
    /// project.
    pub fn relink_source(&mut self, source_id: &str, path: impl AsRef<Path>) -> Result<()> {
        let path = path
            .as_ref()
            .canonicalize()
            .with_context(|| format!("opening relinked media {}", path.as_ref().display()))?;
        let probe = probe_media(&path)?;
        let previous = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        if previous.has_video && !probe.has_video {
            bail!("the selected file does not contain the required video stream");
        }
        if previous.has_audio && !probe.has_audio {
            bail!("the selected file does not contain the required audio stream");
        }
        if previous.still_image != probe.still_image {
            bail!("the selected file is not the same still-image media type as the missing source");
        }
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Media")
            .chars()
            .take(512)
            .collect::<String>();
        let waveform_peaks = if probe.has_audio {
            crate::media_audio::waveform_for_media(&path, probe.duration, 512).unwrap_or_default()
        } else {
            Vec::new()
        };
        let index = self
            .sources
            .iter()
            .position(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        let mut candidate = self.clone();
        candidate.sources[index].path = path;
        candidate.sources[index].nested_composition_path = None;
        candidate.sources[index].proxy_path = None;
        candidate.sources[index].proxy_metadata = None;
        candidate.sources[index].display_name = display_name;
        candidate.sources[index].has_video = probe.has_video;
        candidate.sources[index].has_audio = probe.has_audio;
        candidate.sources[index].duration = if probe.still_image {
            0.0
        } else {
            probe.duration
        };
        candidate.sources[index].width = probe.width;
        candidate.sources[index].height = probe.height;
        candidate.sources[index].still_image = probe.still_image;
        candidate.sources[index].waveform_peaks = waveform_peaks;
        candidate
            .validate()
            .context("relinked media is shorter than an existing clip or otherwise incompatible")?;
        *self = candidate;
        Ok(())
    }

    pub fn set_source_proxy(&mut self, source_id: &str, path: impl AsRef<Path>) -> Result<()> {
        self.attach_source_proxy(source_id, path.as_ref(), false)
    }

    pub fn set_generated_source_proxy(
        &mut self,
        source_id: &str,
        path: impl AsRef<Path>,
    ) -> Result<()> {
        self.attach_source_proxy(source_id, path.as_ref(), true)
    }

    fn attach_source_proxy(&mut self, source_id: &str, path: &Path, generated: bool) -> Result<()> {
        let path = path
            .canonicalize()
            .with_context(|| format!("resolving video proxy {}", path.display()))?;
        let probe = probe_media(&path)?;
        if !probe.has_video || probe.still_image {
            bail!("the selected proxy must contain a video stream");
        }
        let index = self
            .sources
            .iter()
            .position(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        let source = &self.sources[index];
        if !source.has_video
            || source.still_image
            || source.solid_color.is_some()
            || source.nested_composition_path.is_some()
        {
            bail!("selected source cannot use a video proxy");
        }
        if source
            .path
            .canonicalize()
            .is_ok_and(|original| original == path)
        {
            bail!("proxy path must be different from the original media path");
        }
        let tolerance = (1.0 / f64::from(self.fps)) + 0.05;
        if (probe.duration - source.duration).abs() > tolerance {
            bail!("proxy duration does not match the original media duration");
        }
        let (source_width, source_height) = source
            .width
            .zip(source.height)
            .context("original source dimensions are unavailable")?;
        let (proxy_width, proxy_height) = probe
            .width
            .zip(probe.height)
            .context("proxy dimensions are unavailable")?;
        let source_aspect = f64::from(source_width) / f64::from(source_height);
        let proxy_aspect = f64::from(proxy_width) / f64::from(proxy_height);
        if (source_aspect - proxy_aspect).abs() > 0.01 {
            bail!("proxy aspect ratio does not match the original source");
        }
        let source_signature = media_file_signature(&source.path)?;
        let proxy_signature = media_file_signature(&path)?;
        let mut candidate = self.clone();
        candidate.sources[index].proxy_path = Some(path);
        candidate.sources[index].proxy_metadata = Some(VideoProxyMetadata {
            source_id: source.id.clone(),
            source_signature,
            proxy_signature,
            width: proxy_width,
            height: proxy_height,
            duration: probe.duration,
            video_codec: probe.video_codec.unwrap_or_else(|| "unknown".into()),
            generated_by_renderer: generated,
        });
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn clear_source_proxy(&mut self, source_id: &str) -> Result<()> {
        let mut candidate = self.clone();
        let source = candidate
            .sources
            .iter_mut()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        source.proxy_path = None;
        source.proxy_metadata = None;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Remove a renderer-generated proxy file only while its bytes still
    /// match the recorded identity. User-selected proxy files are detached
    /// without deleting their source media.
    pub fn delete_generated_source_proxy(&mut self, source_id: &str) -> Result<bool> {
        let source = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        let (Some(path), Some(metadata)) = (&source.proxy_path, &source.proxy_metadata) else {
            bail!("selected source has no attached video proxy");
        };
        if !metadata.generated_by_renderer {
            bail!("this proxy was not generated by Sono-Renderer; it will only be detached");
        }
        if path.exists() && media_file_signature(path)? != metadata.proxy_signature {
            bail!(
                "proxy file changed since it was generated; refusing to delete user-modified bytes"
            );
        }
        let deleted = match fs::remove_file(path) {
            Ok(()) => true,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                return Err(error).with_context(|| format!("deleting proxy {}", path.display()))
            }
        };
        self.clear_source_proxy(source_id)?;
        Ok(deleted)
    }

    pub fn source_proxy_status(&self, source_id: &str) -> Option<VideoProxyStatus> {
        let source = self.sources.iter().find(|source| source.id == source_id)?;
        let proxy_path = source.proxy_path.as_deref()?;
        let Some(proxy) = &source.proxy_metadata else {
            return Some(VideoProxyStatus::MetadataMissing);
        };
        if proxy.source_id != source.id {
            return Some(VideoProxyStatus::Stale);
        }
        if !proxy_path.is_file() {
            return Some(VideoProxyStatus::ProxyOffline);
        }
        if media_file_signature(proxy_path).ok() != Some(proxy.proxy_signature) {
            return Some(VideoProxyStatus::Stale);
        }
        if source.path.is_file() {
            if media_file_signature(&source.path).ok() != Some(proxy.source_signature) {
                return Some(VideoProxyStatus::Stale);
            }
            Some(VideoProxyStatus::Ready)
        } else {
            Some(VideoProxyStatus::ReadyOriginalOffline)
        }
    }

    /// Add a finalized microphone take through the same source model used by
    /// imported audio. The audio-input subsystem supplies only a WAV path.
    pub fn add_recorded_source(&mut self, path: impl AsRef<Path>) -> Result<String> {
        let path = path
            .as_ref()
            .canonicalize()
            .with_context(|| format!("opening recorded audio {}", path.as_ref().display()))?;
        let (sample_rate, _channels, frames, waveform_peaks) =
            crate::media_audio::inspect_pcm_wav(&path)?;
        let duration = frames as f64 / f64::from(sample_rate);
        if !duration.is_finite() || duration <= 0.0 {
            bail!("recorded audio has an invalid duration");
        }
        let id = unique_id("source");
        let display_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("Voiceover")
            .chars()
            .take(512)
            .collect();
        self.sources.push(MediaSource {
            id: id.clone(),
            path,
            display_name,
            has_video: false,
            has_audio: true,
            duration,
            waveform_peaks,
            ..Default::default()
        });
        self.validate()?;
        Ok(id)
    }

    pub fn add_clip(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        timeline_start: f64,
    ) -> Result<String> {
        self.add_clip_to_track(source_id, kind, timeline_start, None)
    }

    pub fn add_clip_to_track(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        timeline_start: f64,
        track_id: Option<&str>,
    ) -> Result<String> {
        finite_range(timeline_start, 0.0, self.duration, "clip timeline start")?;
        let source = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        let (has_stream, duration) = match kind {
            ClipKind::Video => (source.has_video, source.duration),
            ClipKind::Audio => (source.has_audio, source.duration),
            ClipKind::Text => bail!("use add_text_clip for text layers"),
            ClipKind::Adjustment => bail!("use add_adjustment_clip for adjustment layers"),
        };
        if !has_stream {
            bail!("selected source does not contain the requested stream");
        }
        let duration = if source.still_image || source.solid_color.is_some() {
            (self.duration - timeline_start).max(0.0)
        } else {
            duration.min((self.duration - timeline_start).max(0.0))
        };
        if duration <= 0.0 {
            bail!("there is no remaining composition duration at that timeline position");
        }
        let track_kind = match kind {
            ClipKind::Audio => TrackKind::Audio,
            ClipKind::Video | ClipKind::Text | ClipKind::Adjustment => TrackKind::Visual,
        };
        let selected_track = if let Some(track_id) = track_id {
            let track = self
                .tracks
                .iter()
                .find(|track| track.id == track_id)
                .context("selected track no longer exists")?;
            if track.kind != track_kind {
                bail!("selected track is incompatible with this clip type");
            }
            if track.locked {
                bail!("selected track is locked");
            }
            Some(track.id.clone())
        } else {
            let track = self
                .tracks
                .iter()
                .filter(|track| track.kind == track_kind && !track.locked)
                .min_by_key(|track| (track.order, track.id.as_str()))
                .context("no unlocked compatible track is available")?;
            Some(track.id.clone())
        };
        let mut clip = CompositionClip {
            id: unique_id("clip"),
            name: source.display_name.clone(),
            kind,
            source_id: Some(source_id.to_owned()),
            track_id: selected_track,
            timeline_start,
            duration,
            z_order: self.clips.len() as i32,
            ..Default::default()
        };
        if kind == ClipKind::Video {
            if let (Some(width), Some(height)) = (source.width, source.height) {
                let fit = (f64::from(self.width) / f64::from(width))
                    .min(f64::from(self.height) / f64::from(height));
                clip.scale_x = fit;
                clip.scale_y = fit;
            }
        }
        let id = clip.id.clone();
        self.clips.push(clip);
        self.validate()?;
        Ok(id)
    }

    pub fn add_solid_color_source(&mut self, color: &str) -> Result<String> {
        validate_color(color)?;
        let id = unique_id("solid");
        self.sources.push(MediaSource {
            id: id.clone(),
            display_name: format!("Solid #{color}"),
            has_video: true,
            width: Some(self.width),
            height: Some(self.height),
            solid_color: Some(color.to_ascii_uppercase()),
            ..Default::default()
        });
        Ok(id)
    }

    pub fn add_shape_source(&mut self, kind: GeneratedShapeKind, color: &str) -> Result<String> {
        validate_color(color)?;
        let id = unique_id("shape");
        let name = match kind {
            GeneratedShapeKind::Rectangle => "Rectangle",
            GeneratedShapeKind::Ellipse => "Ellipse",
            GeneratedShapeKind::Line => "Line",
        };
        self.sources.push(MediaSource {
            id: id.clone(),
            display_name: format!("{name} #{color}"),
            has_video: true,
            width: Some(self.width),
            height: Some(self.height),
            solid_color: Some(color.to_ascii_uppercase()),
            generated_shape: Some(match kind {
                GeneratedShapeKind::Line => GeneratedShape {
                    kind,
                    y: 0.49,
                    height: 0.02,
                    ..Default::default()
                },
                _ => GeneratedShape {
                    kind,
                    ..Default::default()
                },
            }),
            ..Default::default()
        });
        self.validate()?;
        Ok(id)
    }

    /// Insert the audio and video streams of one source as a synchronized pair.
    /// Both instances use the same source window and timeline interval.
    pub fn add_linked_av_clip(
        &mut self,
        source_id: &str,
        timeline_start: f64,
    ) -> Result<(String, String)> {
        let source = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        if !source.has_video || !source.has_audio {
            bail!("linked insertion requires a source with both video and audio streams");
        }
        let mut candidate = self.clone();
        let video_id = candidate.add_clip(source_id, ClipKind::Video, timeline_start)?;
        let audio_id = candidate.add_clip(source_id, ClipKind::Audio, timeline_start)?;
        let link_id = unique_id("av-link");
        for clip in &mut candidate.clips {
            if clip.id == video_id || clip.id == audio_id {
                clip.link_group_id = Some(link_id.clone());
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok((video_id, audio_id))
    }

    /// Insert a source window on one track. `source_out` is exclusive source
    /// time; still images use an explicit duration with a zero source in-point.
    pub fn add_clip_from_range(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
        track_id: Option<&str>,
    ) -> Result<String> {
        let source = self
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("selected media source no longer exists")?;
        if !source_in.is_finite() || !source_out.is_finite() || source_out <= source_in {
            bail!("source Out must be later than source In");
        }
        if source.still_image || source.solid_color.is_some() {
            if source_in != 0.0 {
                bail!("still and generated visual sources have a zero source In point");
            }
        } else if source_in < 0.0 || source_out > source.duration + 1.0e-6 {
            bail!("selected source range is outside the media duration");
        }
        let duration = source_out - source_in;
        if timeline_start < 0.0 || timeline_start + duration > self.duration + 1.0e-6 {
            bail!("selected source range does not fit at the timeline playhead");
        }
        let mut candidate = self.clone();
        let id = candidate.add_clip_to_track(source_id, kind, timeline_start, track_id)?;
        let clip = candidate
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("inserted clip disappeared")?;
        clip.source_start = source_in;
        clip.duration = duration;
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    /// Insert a range as a sequence edit: every track opens the same gap,
    /// clips crossing the playhead are split, and later clips/markers move.
    pub fn insert_clip_from_range(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
        track_id: Option<&str>,
    ) -> Result<String> {
        if !source_in.is_finite() || !source_out.is_finite() || source_out <= source_in {
            bail!("source Out must be later than source In");
        }
        let mut candidate = self.clone();
        let duration = source_out - source_in;
        candidate.insert_time_gap(timeline_start, duration)?;
        let id = candidate.add_clip_from_range(
            source_id,
            kind,
            timeline_start,
            source_in,
            source_out,
            track_id,
        )?;
        *self = candidate;
        Ok(id)
    }

    pub fn append_clip_from_range(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        source_in: f64,
        source_out: f64,
        track_id: Option<&str>,
    ) -> Result<String> {
        self.insert_clip_from_range(
            source_id,
            kind,
            self.duration,
            source_in,
            source_out,
            track_id,
        )
    }

    pub fn insert_linked_av_range(
        &mut self,
        source_id: &str,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
    ) -> Result<(String, String)> {
        if !source_in.is_finite() || !source_out.is_finite() || source_out <= source_in {
            bail!("source Out must be later than source In");
        }
        let mut candidate = self.clone();
        candidate.insert_time_gap(timeline_start, source_out - source_in)?;
        let pair =
            candidate.add_linked_av_range(source_id, timeline_start, source_in, source_out)?;
        *self = candidate;
        Ok(pair)
    }

    pub fn append_linked_av_range(
        &mut self,
        source_id: &str,
        source_in: f64,
        source_out: f64,
    ) -> Result<(String, String)> {
        self.insert_linked_av_range(source_id, self.duration, source_in, source_out)
    }

    /// Replace the selected interval on the destination track. Any partially
    /// covered clips are trimmed and their surviving tail keeps its source
    /// time. Other tracks are unchanged. Linked clips must be unlinked first.
    pub fn overwrite_clip_from_range(
        &mut self,
        source_id: &str,
        kind: ClipKind,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
        track_id: Option<&str>,
    ) -> Result<String> {
        if !source_in.is_finite() || !source_out.is_finite() || source_out <= source_in {
            bail!("source Out must be later than source In");
        }
        let duration = source_out - source_in;
        let end = timeline_start + duration;
        finite_range(timeline_start, 0.0, self.duration, "overwrite position")?;
        finite_range(end, 0.0, self.duration, "overwrite end")?;
        let track_kind = match kind {
            ClipKind::Audio => TrackKind::Audio,
            ClipKind::Video | ClipKind::Text | ClipKind::Adjustment => TrackKind::Visual,
        };
        let destination = if let Some(id) = track_id {
            self.tracks
                .iter()
                .find(|track| track.id == id && track.kind == track_kind)
                .context("overwrite destination track is missing or incompatible")?
        } else {
            self.tracks
                .iter()
                .filter(|track| track.kind == track_kind && !track.locked)
                .min_by_key(|track| (track.order, track.id.as_str()))
                .context("no unlocked overwrite destination track is available")?
        };
        if destination.locked {
            bail!("overwrite destination track is locked");
        }
        let destination_id = destination.id.clone();
        let overlaps: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.track_id.as_deref() == Some(destination_id.as_str())
                    && clip.timeline_start < end
                    && clip.timeline_start + clip.duration > timeline_start
            })
            .map(|clip| clip.id.clone())
            .collect();
        if overlaps.iter().any(|id| {
            self.clips
                .iter()
                .find(|clip| &clip.id == id)
                .is_some_and(|clip| clip.link_group_id.is_some())
        }) {
            bail!("unlink overlapping linked A/V clips before overwriting this track");
        }

        let mut candidate = self.clone();
        for id in overlaps {
            let Some(index) = candidate.clips.iter().position(|clip| clip.id == id) else {
                continue;
            };
            let old = candidate.clips[index].clone();
            let old_end = old.timeline_start + old.duration;
            let keep_left = old.timeline_start < timeline_start;
            let keep_right = old_end > end;
            match (keep_left, keep_right) {
                (true, true) => {
                    let left_duration = timeline_start - old.timeline_start;
                    candidate.clips[index].duration = left_duration;
                    let mut tail = old.clone();
                    tail.id = unique_id("clip");
                    tail.timeline_start = end;
                    tail.duration = old_end - end;
                    let generated = tail.source_id.as_deref().and_then(|source_id| {
                        candidate
                            .sources
                            .iter()
                            .find(|source| source.id == source_id)
                    });
                    let unbounded = generated
                        .is_some_and(|source| source.still_image || source.solid_color.is_some());
                    candidate.clips[index].source_start =
                        source_start_for_subclip(&old, 0.0, left_duration, unbounded);
                    tail.source_start = source_start_for_subclip(
                        &old,
                        end - old.timeline_start,
                        tail.duration,
                        unbounded,
                    );
                    candidate.clips[index].fade_out = candidate.clips[index]
                        .fade_out
                        .min(candidate.clips[index].duration);
                    tail.fade_in = tail.fade_in.min(tail.duration);
                    tail.fade_out = tail.fade_out.min(tail.duration);
                    candidate.clips.push(tail);
                }
                (true, false) => {
                    let left_duration = timeline_start - old.timeline_start;
                    candidate.clips[index].duration = left_duration;
                    let unbounded = old
                        .source_id
                        .as_deref()
                        .and_then(|source_id| {
                            candidate
                                .sources
                                .iter()
                                .find(|source| source.id == source_id)
                        })
                        .is_some_and(|source| source.still_image || source.solid_color.is_some());
                    candidate.clips[index].source_start =
                        source_start_for_subclip(&old, 0.0, left_duration, unbounded);
                    candidate.clips[index].fade_in = candidate.clips[index]
                        .fade_in
                        .min(candidate.clips[index].duration);
                    candidate.clips[index].fade_out = candidate.clips[index]
                        .fade_out
                        .min(candidate.clips[index].duration);
                }
                (false, true) => {
                    candidate.clips[index].timeline_start = end;
                    candidate.clips[index].duration = old_end - end;
                    let generated = old.source_id.as_deref().and_then(|source_id| {
                        candidate
                            .sources
                            .iter()
                            .find(|source| source.id == source_id)
                    });
                    let unbounded = generated
                        .is_some_and(|source| source.still_image || source.solid_color.is_some());
                    candidate.clips[index].source_start = source_start_for_subclip(
                        &old,
                        end - old.timeline_start,
                        candidate.clips[index].duration,
                        unbounded,
                    );
                    candidate.clips[index].fade_in = candidate.clips[index]
                        .fade_in
                        .min(candidate.clips[index].duration);
                    candidate.clips[index].fade_out = candidate.clips[index]
                        .fade_out
                        .min(candidate.clips[index].duration);
                }
                (false, false) => candidate.clips.retain(|clip| clip.id != id),
            }
        }
        let id = candidate.add_clip_from_range(
            source_id,
            kind,
            timeline_start,
            source_in,
            source_out,
            Some(&destination_id),
        )?;
        *self = candidate;
        Ok(id)
    }

    pub fn overwrite_linked_av_range(
        &mut self,
        source_id: &str,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
    ) -> Result<(String, String)> {
        let mut candidate = self.clone();
        let video = candidate.overwrite_clip_from_range(
            source_id,
            ClipKind::Video,
            timeline_start,
            source_in,
            source_out,
            None,
        )?;
        let audio = candidate.overwrite_clip_from_range(
            source_id,
            ClipKind::Audio,
            timeline_start,
            source_in,
            source_out,
            None,
        )?;
        let link_id = unique_id("av-link");
        for clip in &mut candidate.clips {
            if clip.id == video || clip.id == audio {
                clip.link_group_id = Some(link_id.clone());
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok((video, audio))
    }

    fn insert_time_gap(&mut self, at: f64, duration: f64) -> Result<()> {
        finite_range(at, 0.0, self.duration, "insert position")?;
        finite_range(
            duration,
            1.0 / f64::from(self.fps),
            86_400.0,
            "insert duration",
        )?;
        finite_range(
            self.duration + duration,
            0.1,
            86_400.0,
            "extended composition duration",
        )?;
        let mut candidate = self.clone();
        candidate.duration += duration;

        let mut crossing_ids = Vec::new();
        let mut crossing_links = HashSet::new();
        for clip in &candidate.clips {
            if clip.timeline_start < at && clip.timeline_start + clip.duration > at {
                if let Some(link_id) = &clip.link_group_id {
                    if crossing_links.insert(link_id.clone()) {
                        crossing_ids.push(clip.id.clone());
                    }
                } else {
                    crossing_ids.push(clip.id.clone());
                }
            }
        }
        for id in crossing_ids {
            candidate.split_clip(&id, at)?;
        }
        if candidate.clips.iter().any(|clip| {
            clip.timeline_start >= at
                && candidate
                    .track_for_clip(clip)
                    .is_some_and(|track| track.locked)
        }) {
            bail!("a locked track prevents inserting a sequence gap");
        }
        for clip in &mut candidate.clips {
            if clip.timeline_start >= at - 1.0e-6 {
                clip.timeline_start += duration;
            }
        }
        for marker in &mut candidate.markers {
            if marker.time >= at {
                marker.time += duration;
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Insert the selected video and audio source window together as one
    /// synchronized linked A/V pair.
    pub fn add_linked_av_range(
        &mut self,
        source_id: &str,
        timeline_start: f64,
        source_in: f64,
        source_out: f64,
    ) -> Result<(String, String)> {
        let mut candidate = self.clone();
        let (video_id, audio_id) = candidate.add_linked_av_clip(source_id, timeline_start)?;
        let duration = source_out - source_in;
        for clip in &mut candidate.clips {
            if clip.id == video_id || clip.id == audio_id {
                clip.source_start = source_in;
                clip.duration = duration;
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok((video_id, audio_id))
    }

    /// Create one user-edit group from selected clips. Existing groups are
    /// merged as complete units, and linked A/V partners are always included.
    pub fn group_clips(&mut self, clip_ids: &[String]) -> Result<String> {
        let mut selected = HashSet::new();
        let mut existing_groups = HashSet::new();
        for id in clip_ids {
            let clip = self
                .clips
                .iter()
                .find(|clip| clip.id == *id)
                .with_context(|| format!("selected clip {id:?} no longer exists"))?;
            selected.insert(id.as_str());
            if let Some(group_id) = clip.group_id.as_deref() {
                existing_groups.insert(group_id);
            }
        }
        let mut affected = self
            .clips
            .iter()
            .filter(|clip| {
                selected.contains(clip.id.as_str())
                    || clip
                        .group_id
                        .as_deref()
                        .is_some_and(|id| existing_groups.contains(id))
            })
            .map(|clip| clip.id.as_str())
            .collect::<HashSet<_>>();
        let linked_ids = self
            .clips
            .iter()
            .filter(|clip| affected.contains(clip.id.as_str()))
            .filter_map(|clip| clip.link_group_id.as_deref())
            .collect::<HashSet<_>>();
        affected.extend(
            self.clips
                .iter()
                .filter(|clip| {
                    clip.link_group_id
                        .as_deref()
                        .is_some_and(|id| linked_ids.contains(id))
                })
                .map(|clip| clip.id.as_str()),
        );

        let units = self
            .clips
            .iter()
            .filter(|clip| affected.contains(clip.id.as_str()))
            .map(|clip| {
                clip.group_id
                    .as_deref()
                    .map(|id| format!("group:{id}"))
                    .or_else(|| clip.link_group_id.as_deref().map(|id| format!("link:{id}")))
                    .unwrap_or_else(|| format!("clip:{}", clip.id))
            })
            .collect::<HashSet<_>>();
        if units.len() < 2 {
            bail!("select at least two independent clips or groups to group them");
        }
        if self
            .clips
            .iter()
            .filter(|clip| affected.contains(clip.id.as_str()))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents grouping these clips");
        }

        let group_id = unique_id("group");
        let mut candidate = self.clone();
        for clip in &mut candidate.clips {
            if affected.contains(clip.id.as_str()) {
                clip.group_id = Some(group_id.clone());
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok(group_id)
    }

    pub fn ungroup_clip(&mut self, clip_id: &str) -> Result<()> {
        let group_id = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?
            .group_id
            .clone()
            .context("selected clip is not in a user group")?;
        if self
            .clips
            .iter()
            .filter(|clip| clip.group_id.as_deref() == Some(group_id.as_str()))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents ungrouping these clips");
        }
        for clip in &mut self.clips {
            if clip.group_id.as_deref() == Some(group_id.as_str()) {
                clip.group_id = None;
            }
        }
        self.validate()
    }

    pub fn unlink_clip(&mut self, clip_id: &str) -> Result<()> {
        let selected = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let link_id = selected.link_group_id.clone();
        if self
            .clips
            .iter()
            .filter(|clip| {
                clip.id == clip_id
                    || link_id
                        .as_deref()
                        .is_some_and(|id| clip.link_group_id.as_deref() == Some(id))
            })
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents unlinking this clip");
        }
        if let Some(link_id) = link_id {
            for clip in &mut self.clips {
                if clip.link_group_id.as_deref() == Some(link_id.as_str()) {
                    clip.link_group_id = None;
                }
            }
        }
        Ok(())
    }

    pub fn remove_clip(&mut self, clip_id: &str) -> Result<()> {
        let clip = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let link_id = clip.link_group_id.clone();
        let group_id = clip.group_id.clone();
        let linked_group_ids = self
            .clips
            .iter()
            .filter(|candidate| {
                group_id
                    .as_deref()
                    .is_some_and(|id| candidate.group_id.as_deref() == Some(id))
            })
            .filter_map(|candidate| candidate.link_group_id.clone())
            .collect::<HashSet<_>>();
        let ids: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.id == clip_id
                    || group_id
                        .as_deref()
                        .is_some_and(|id| clip.group_id.as_deref() == Some(id))
                    || link_id
                        .as_deref()
                        .is_some_and(|link_id| clip.link_group_id.as_deref() == Some(link_id))
                    || clip
                        .link_group_id
                        .as_ref()
                        .is_some_and(|id| linked_group_ids.contains(id))
            })
            .map(|clip| clip.id.clone())
            .collect::<Vec<_>>();
        if self
            .clips
            .iter()
            .filter(|clip| ids.contains(&clip.id))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents deleting this clip");
        }
        self.clips.retain(|clip| !ids.contains(&clip.id));
        self.transitions.retain(|transition| {
            !ids.contains(&transition.from_clip_id) && !ids.contains(&transition.to_clip_id)
        });
        Ok(())
    }

    pub fn move_clip_to(&mut self, clip_id: &str, new_start: f64) -> Result<()> {
        let selected = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        finite_range(
            new_start,
            0.0,
            self.duration - selected.duration,
            "clip timeline start",
        )?;
        let old_start = selected.timeline_start;
        let link_id = selected.link_group_id.clone();
        let group_id = selected.group_id.clone();
        let grouped_link_ids = self
            .clips
            .iter()
            .filter(|clip| {
                group_id
                    .as_deref()
                    .is_some_and(|id| clip.group_id.as_deref() == Some(id))
            })
            .filter_map(|clip| clip.link_group_id.as_deref())
            .collect::<HashSet<_>>();
        let ids: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.id == clip_id
                    || group_id
                        .as_deref()
                        .is_some_and(|id| clip.group_id.as_deref() == Some(id))
                    || link_id
                        .as_deref()
                        .is_some_and(|link_id| clip.link_group_id.as_deref() == Some(link_id))
                    || clip
                        .link_group_id
                        .as_deref()
                        .is_some_and(|id| grouped_link_ids.contains(id))
            })
            .map(|clip| clip.id.clone())
            .collect();
        if self
            .clips
            .iter()
            .filter(|clip| ids.contains(&clip.id))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents moving this clip");
        }
        let delta = new_start - old_start;
        let mut candidate = self.clone();
        for clip in &mut candidate.clips {
            if ids.contains(&clip.id) {
                clip.timeline_start += delta;
            }
        }
        let mut adjusted = Vec::new();
        for transition in &candidate.transitions {
            if !ids.contains(&transition.from_clip_id) && !ids.contains(&transition.to_clip_id) {
                continue;
            }
            let from = candidate
                .clips
                .iter()
                .find(|clip| clip.id == transition.from_clip_id)
                .context("transition outgoing clip no longer exists")?;
            let to = candidate
                .clips
                .iter()
                .find(|clip| clip.id == transition.to_clip_id)
                .context("transition incoming clip no longer exists")?;
            let shared_start = from.timeline_start.max(to.timeline_start);
            let shared_end =
                (from.timeline_start + from.duration).min(to.timeline_start + to.duration);
            if shared_end - shared_start < transition.duration {
                bail!("moving the clip would break its transition; move both clips or remove the transition");
            }
            let center = transition.start_time + transition.duration / 2.0;
            let start = (center - transition.duration / 2.0)
                .clamp(shared_start, shared_end - transition.duration);
            adjusted.push((transition.id.clone(), start));
        }
        for transition in &mut candidate.transitions {
            if let Some((_, start)) = adjusted.iter().find(|(id, _)| *id == transition.id) {
                transition.start_time = *start;
            }
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn add_visual_effect(&mut self, clip_id: &str, kind: VisualEffectKind) -> Result<String> {
        let mut candidate = self.clone();
        let clip_index = candidate
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        if !matches!(
            candidate.clips[clip_index].kind,
            ClipKind::Video | ClipKind::Adjustment
        ) {
            bail!("visual effects can only be attached to video or adjustment clips");
        }
        if candidate
            .track_for_clip(&candidate.clips[clip_index])
            .is_some_and(|track| track.locked)
        {
            bail!("a locked track prevents editing this clip");
        }
        let effect = VisualEffectInstance::new(kind);
        validate_visual_effect(&effect.kind)?;
        let id = effect.id.clone();
        candidate.clips[clip_index].effects.push(effect);
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    pub fn remove_visual_effect(&mut self, clip_id: &str, effect_id: &str) -> Result<()> {
        let mut candidate = self.clone();
        let clip_index = candidate
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        if candidate
            .track_for_clip(&candidate.clips[clip_index])
            .is_some_and(|track| track.locked)
        {
            bail!("a locked track prevents editing this clip");
        }
        let effects = &mut candidate.clips[clip_index].effects;
        let effect_index = effects
            .iter()
            .position(|effect| effect.id == effect_id)
            .context("selected visual effect no longer exists")?;
        effects.remove(effect_index);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn move_visual_effect(
        &mut self,
        clip_id: &str,
        effect_id: &str,
        direction: isize,
    ) -> Result<()> {
        if direction != -1 && direction != 1 {
            bail!("visual effect order direction must be -1 or 1");
        }
        let mut candidate = self.clone();
        let clip_index = candidate
            .clips
            .iter()
            .position(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        if candidate
            .track_for_clip(&candidate.clips[clip_index])
            .is_some_and(|track| track.locked)
        {
            bail!("a locked track prevents editing this clip");
        }
        let effects = &mut candidate.clips[clip_index].effects;
        let index = effects
            .iter()
            .position(|effect| effect.id == effect_id)
            .context("selected visual effect no longer exists")?;
        let target = index as isize + direction;
        if !(0..effects.len() as isize).contains(&target) {
            return Ok(());
        }
        effects.swap(index, target as usize);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn set_effect_keyframe(
        &mut self,
        clip_id: &str,
        effect_id: &str,
        parameter: EffectParameter,
        time: f64,
        value: f64,
        easing: Easing,
    ) -> Result<()> {
        finite_range(time, 0.0, self.duration, "effect keyframe time")?;
        let mut candidate = self.clone();
        let locked = candidate
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")
            .and_then(|clip| {
                Ok(candidate
                    .track_for_clip(clip)
                    .is_some_and(|track| track.locked))
            })?;
        if locked {
            bail!("a locked track prevents editing this effect");
        }
        let clip = candidate
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let effect = clip
            .effects
            .iter_mut()
            .find(|effect| effect.id == effect_id)
            .context("selected visual effect no longer exists")?;
        let (minimum, maximum) = effect_parameter_range(&effect.kind, parameter)
            .context("this visual effect parameter cannot be animated")?;
        finite_range(value, minimum, maximum, "effect keyframe value")?;
        let keys = effect.animated_parameters.entry(parameter).or_default();
        if let Some(key) = keys
            .iter_mut()
            .find(|key| (key.time - time).abs() < 0.000_001)
        {
            key.value = value;
            key.easing_to_next = easing;
        } else {
            keys.push(ScalarKeyframe {
                time,
                value,
                easing_to_next: easing,
            });
            keys.sort_by(|left, right| left.time.total_cmp(&right.time));
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn edit_effect_keyframe(
        &mut self,
        clip_id: &str,
        effect_id: &str,
        parameter: EffectParameter,
        original_time: f64,
        time: f64,
        value: f64,
        easing: Easing,
    ) -> Result<()> {
        finite_range(time, 0.0, self.duration, "effect keyframe time")?;
        let mut candidate = self.clone();
        let locked = candidate
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")
            .map(|clip| {
                candidate
                    .track_for_clip(clip)
                    .is_some_and(|track| track.locked)
            })?;
        if locked {
            bail!("a locked track prevents editing this effect");
        }
        let clip = candidate
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let effect = clip
            .effects
            .iter_mut()
            .find(|effect| effect.id == effect_id)
            .context("selected visual effect no longer exists")?;
        let (minimum, maximum) = effect_parameter_range(&effect.kind, parameter)
            .context("this visual effect parameter cannot be animated")?;
        finite_range(value, minimum, maximum, "effect keyframe value")?;
        let keys = effect
            .animated_parameters
            .get_mut(&parameter)
            .context("selected effect parameter has no keyframes")?;
        let index = keys
            .iter()
            .position(|key| (key.time - original_time).abs() < 0.000_001)
            .context("selected effect keyframe no longer exists")?;
        if keys
            .iter()
            .enumerate()
            .any(|(other, key)| other != index && (key.time - time).abs() < 0.000_001)
        {
            bail!("effect keyframes cannot share the same timestamp");
        }
        keys[index] = ScalarKeyframe {
            time,
            value,
            easing_to_next: easing,
        };
        keys.sort_by(|left, right| left.time.total_cmp(&right.time));
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    pub fn remove_effect_keyframe(
        &mut self,
        clip_id: &str,
        effect_id: &str,
        parameter: EffectParameter,
        time: f64,
    ) -> Result<bool> {
        let mut candidate = self.clone();
        let locked = candidate
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")
            .map(|clip| {
                candidate
                    .track_for_clip(clip)
                    .is_some_and(|track| track.locked)
            })?;
        if locked {
            bail!("a locked track prevents editing this effect");
        }
        let clip = candidate
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let effect = clip
            .effects
            .iter_mut()
            .find(|effect| effect.id == effect_id)
            .context("selected visual effect no longer exists")?;
        let Some(keys) = effect.animated_parameters.get_mut(&parameter) else {
            return Ok(false);
        };
        let previous_len = keys.len();
        keys.retain(|key| (key.time - time).abs() >= 0.000_001);
        let removed = keys.len() != previous_len;
        if keys.is_empty() {
            effect.animated_parameters.remove(&parameter);
        }
        candidate.validate()?;
        *self = candidate;
        Ok(removed)
    }

    /// Change the source window without moving or resizing the timeline clip.
    pub fn slip_clip_by(&mut self, clip_id: &str, source_delta: f64) -> Result<()> {
        if !source_delta.is_finite() {
            bail!("slip amount must be finite");
        }
        let selected = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        let link_id = selected.link_group_id.clone();
        let ids: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.id == clip_id
                    || link_id
                        .as_deref()
                        .is_some_and(|link_id| clip.link_group_id.as_deref() == Some(link_id))
            })
            .map(|clip| clip.id.clone())
            .collect();
        if self
            .clips
            .iter()
            .filter(|clip| ids.contains(&clip.id))
            .any(|clip| self.track_for_clip(clip).is_some_and(|track| track.locked))
        {
            bail!("a locked track prevents slipping this clip");
        }
        for clip in &self.clips {
            if ids.contains(&clip.id) {
                let source = clip
                    .source_id
                    .as_deref()
                    .and_then(|id| self.sources.iter().find(|source| source.id == id))
                    .context("selected clip no longer has a source")?;
                if source.still_image || source.solid_color.is_some() {
                    bail!("generated and still-image clips have no source window to slip");
                }
                finite_range(
                    clip.source_start + source_delta,
                    0.0,
                    source.duration - clip.duration * clip.playback_rate,
                    "slipped source in-point",
                )?;
            }
        }
        for clip in &mut self.clips {
            if ids.contains(&clip.id) {
                clip.source_start += source_delta;
            }
        }
        Ok(())
    }

    /// Remove the clip's timeline interval and close the gap on its own track.
    /// Linked partners are removed with it; other tracks are left untouched.
    pub fn ripple_delete_clip(&mut self, clip_id: &str) -> Result<()> {
        let mut candidate = self.clone();
        candidate.ripple_delete_clip_in_place(clip_id)?;
        *self = candidate;
        Ok(())
    }

    fn ripple_delete_clip_in_place(&mut self, clip_id: &str) -> Result<()> {
        let selected = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?
            .clone();
        let track_id = selected
            .track_id
            .clone()
            .context("ripple delete requires an explicitly assigned track")?;
        let cut_start = selected.timeline_start;
        let cut_end = selected.timeline_start + selected.duration;
        let shift = selected.duration;
        self.remove_clip(clip_id)?;

        let intersecting: Vec<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.track_id.as_deref() == Some(track_id.as_str())
                    && clip.timeline_start < cut_end
                    && clip.timeline_start + clip.duration > cut_start
            })
            .map(|clip| clip.id.clone())
            .collect();
        for original_id in intersecting {
            let Some(original) = self
                .clips
                .iter()
                .find(|clip| clip.id == original_id)
                .cloned()
            else {
                continue;
            };
            let mut segment_id = original_id.clone();
            if original.timeline_start < cut_start
                && original.timeline_start + original.duration > cut_start
            {
                segment_id = self.split_clip(&original_id, cut_start)?;
            }
            let segment = self
                .clips
                .iter()
                .find(|clip| clip.id == segment_id)
                .context("ripple segment disappeared after split")?
                .clone();
            if segment.timeline_start < cut_end
                && segment.timeline_start + segment.duration > cut_end
            {
                self.split_clip(&segment_id, cut_end)?;
            }
            if self
                .clips
                .iter()
                .find(|clip| clip.id == segment_id)
                .is_some_and(|clip| {
                    clip.timeline_start >= cut_start - 1.0e-6
                        && clip.timeline_start + clip.duration <= cut_end + 1.0e-6
                })
            {
                self.remove_clip(&segment_id)?;
            }
        }

        let moving_link_ids: HashSet<_> = self
            .clips
            .iter()
            .filter(|clip| {
                clip.track_id.as_deref() == Some(track_id.as_str())
                    && clip.timeline_start >= cut_end - 1.0e-6
            })
            .filter_map(|clip| clip.link_group_id.clone())
            .collect();
        if self.clips.iter().any(|clip| {
            ((clip.track_id.as_deref() == Some(track_id.as_str())
                && clip.timeline_start >= cut_end - 1.0e-6)
                || (clip
                    .link_group_id
                    .as_ref()
                    .is_some_and(|id| moving_link_ids.contains(id))
                    && clip.track_id.as_deref() != Some(track_id.as_str())))
                && self.track_for_clip(clip).is_some_and(|track| track.locked)
        }) {
            bail!("a locked track prevents ripple delete");
        }
        for clip in &mut self.clips {
            let on_ripple_track = clip.track_id.as_deref() == Some(track_id.as_str());
            let linked_partner = clip
                .link_group_id
                .as_ref()
                .is_some_and(|id| moving_link_ids.contains(id));
            if (on_ripple_track && clip.timeline_start >= cut_end - 1.0e-6)
                || (linked_partner && !on_ripple_track)
            {
                clip.timeline_start -= shift;
            }
        }
        self.markers
            .retain(|marker| marker.time < cut_start || marker.time >= cut_end);
        for marker in &mut self.markers {
            if marker.time >= cut_end {
                marker.time -= shift;
            }
        }
        self.validate()
    }

    pub fn add_marker(&mut self, time: f64, label: impl Into<String>) -> Result<String> {
        finite_range(time, 0.0, self.duration, "timeline marker time")?;
        let marker = CompositionMarker {
            id: unique_id("marker"),
            time,
            label: label.into(),
            ..Default::default()
        };
        if marker.label.chars().count() > 200 {
            bail!("timeline marker label is too long");
        }
        let id = marker.id.clone();
        self.markers.push(marker);
        self.validate()?;
        Ok(id)
    }

    pub fn add_text_clip(
        &mut self,
        text: impl Into<String>,
        timeline_start: f64,
        duration: f64,
    ) -> Result<String> {
        let track_id = self
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual && !track.locked)
            .min_by_key(|track| (track.order, track.id.as_str()))
            .map(|track| track.id.clone())
            .context("no unlocked visual track is available")?;
        let clip = CompositionClip {
            id: unique_id("clip"),
            name: "Text".into(),
            kind: ClipKind::Text,
            track_id: Some(track_id),
            text: text.into(),
            timeline_start,
            duration,
            z_order: self.clips.len() as i32,
            ..Default::default()
        };
        let id = clip.id.clone();
        self.clips.push(clip);
        self.validate()?;
        Ok(id)
    }

    pub fn add_adjustment_clip(&mut self, timeline_start: f64, duration: f64) -> Result<String> {
        finite_range(timeline_start, 0.0, self.duration, "adjustment start")?;
        finite_range(
            duration,
            1.0 / f64::from(self.fps),
            self.duration,
            "adjustment duration",
        )?;
        finite_range(
            timeline_start + duration,
            0.0,
            self.duration,
            "adjustment end",
        )?;
        let track_id = self
            .tracks
            .iter()
            .filter(|track| track.kind == TrackKind::Visual && !track.locked)
            .min_by_key(|track| (track.order, track.id.as_str()))
            .map(|track| track.id.clone())
            .context("no unlocked visual track is available")?;
        let clip = CompositionClip {
            id: unique_id("adjustment"),
            name: "Adjustment layer".into(),
            kind: ClipKind::Adjustment,
            track_id: Some(track_id),
            timeline_start,
            duration,
            z_order: self.clips.len() as i32,
            ..Default::default()
        };
        let id = clip.id.clone();
        let mut candidate = self.clone();
        candidate.clips.push(clip);
        candidate.validate()?;
        *self = candidate;
        Ok(id)
    }

    pub fn add_title_from_preset(
        &mut self,
        preset_id: &str,
        timeline_start: f64,
        duration: f64,
    ) -> Result<String> {
        let preset = self
            .title_presets
            .iter()
            .find(|preset| preset.id == preset_id)
            .context("selected title preset no longer exists")?
            .clone();
        let id = self.add_text_clip(preset.sample_text.clone(), timeline_start, duration)?;
        let clip = self
            .clips
            .iter_mut()
            .find(|clip| clip.id == id)
            .context("new title clip was not created")?;
        let scale = f64::from(self.width) / 1080.0;
        clip.name = preset.name.clone();
        clip.x = preset.x * f64::from(self.width);
        clip.y = preset.y * f64::from(self.height);
        clip.font_size = (f64::from(preset.font_size) * scale)
            .round()
            .clamp(1.0, 512.0) as u32;
        clip.text_color = preset.text_color;
        clip.outline_color = preset.outline_color;
        clip.outline_width = preset.outline_width * scale;
        clip.shadow_x = preset.shadow_x * scale;
        clip.shadow_y = preset.shadow_y * scale;
        clip.text_alignment = preset.text_alignment;
        clip.text_background_enabled = preset.text_background_enabled;
        clip.text_background_color = preset.text_background_color;
        clip.text_background_opacity = preset.text_background_opacity;
        clip.text_background_padding = (f64::from(preset.text_background_padding) * scale)
            .round()
            .clamp(0.0, 256.0) as u32;
        self.validate()?;
        Ok(id)
    }

    pub fn save_title_preset_from_clip(&mut self, clip_id: &str, name: &str) -> Result<String> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            bail!("title preset name must contain 1..=64 characters");
        }
        if self
            .title_presets
            .iter()
            .any(|preset| preset.name.eq_ignore_ascii_case(name))
        {
            bail!("a title preset with that name already exists");
        }
        if self.title_presets.len() >= 256 {
            bail!("a composition can contain at most 256 title presets");
        }
        let clip = self
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .context("selected clip no longer exists")?;
        if clip.kind != ClipKind::Text {
            bail!("only a text clip can be saved as a title preset");
        }
        let preset = TitlePreset {
            id: unique_id("title-preset"),
            name: name.into(),
            sample_text: clip.text.clone(),
            x: clip.x / f64::from(self.width),
            y: clip.y / f64::from(self.height),
            font_size: (f64::from(clip.font_size) * 1080.0 / f64::from(self.width))
                .round()
                .clamp(1.0, 512.0) as u32,
            text_color: clip.text_color.clone(),
            outline_color: clip.outline_color.clone(),
            outline_width: clip.outline_width * 1080.0 / f64::from(self.width),
            shadow_x: clip.shadow_x * 1080.0 / f64::from(self.width),
            shadow_y: clip.shadow_y * 1080.0 / f64::from(self.width),
            text_alignment: clip.text_alignment,
            text_background_enabled: clip.text_background_enabled,
            text_background_color: clip.text_background_color.clone(),
            text_background_opacity: clip.text_background_opacity,
            text_background_padding: (f64::from(clip.text_background_padding) * 1080.0
                / f64::from(self.width))
            .round()
            .clamp(0.0, 256.0) as u32,
            built_in: false,
        };
        let id = preset.id.clone();
        self.title_presets.push(preset);
        if let Err(error) = self.validate() {
            self.title_presets.retain(|preset| preset.id != id);
            return Err(error);
        }
        Ok(id)
    }

    pub fn remove_title_preset(&mut self, preset_id: &str) -> Result<()> {
        let preset = self
            .title_presets
            .iter()
            .find(|preset| preset.id == preset_id)
            .context("selected title preset no longer exists")?;
        if preset.built_in {
            bail!("built-in title presets cannot be removed");
        }
        self.title_presets.retain(|preset| preset.id != preset_id);
        Ok(())
    }

    pub fn duplicate_clip(&mut self, id: &str) -> Result<String> {
        let mut clip = self
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .context("selected clip no longer exists")?
            .clone();
        if self.track_for_clip(&clip).is_some_and(|track| track.locked) {
            bail!("a locked track prevents duplicating this clip");
        }
        clip.id = unique_id("clip");
        clip.name = format!("{} copy", clip.name);
        clip.link_group_id = None;
        clip.group_id = None;
        for effect in &mut clip.effects {
            effect.id = unique_id("effect");
        }
        clip.timeline_start =
            (clip.timeline_start + clip.duration).min((self.duration - clip.duration).max(0.0));
        clip.z_order = self
            .clips
            .iter()
            .map(|item| item.z_order)
            .max()
            .unwrap_or(-1)
            + 1;
        if clip.kind == ClipKind::Video {
            clip.include_audio = false;
        }
        let new_id = clip.id.clone();
        self.clips.push(clip);
        self.validate()?;
        Ok(new_id)
    }

    pub fn paste_clip(&mut self, source: &CompositionClip) -> Result<String> {
        if let Some(source_id) = source.source_id.as_deref() {
            if !self.sources.iter().any(|item| item.id == source_id) {
                bail!("copied clip refers to a media source that is no longer in this project");
            }
        }
        let mut clip = source.clone();
        if self.track_for_clip(&clip).is_some_and(|track| track.locked) {
            bail!("a locked track prevents pasting this clip");
        }
        clip.id = unique_id("clip");
        clip.name = format!("{} copy", clip.name);
        clip.link_group_id = None;
        clip.group_id = None;
        for effect in &mut clip.effects {
            effect.id = unique_id("effect");
        }
        clip.timeline_start =
            (clip.timeline_start + clip.duration).min((self.duration - clip.duration).max(0.0));
        clip.z_order = self
            .clips
            .iter()
            .map(|item| item.z_order)
            .max()
            .unwrap_or(-1)
            .saturating_add(1);
        if clip.kind == ClipKind::Video {
            clip.include_audio = false;
        }
        let id = clip.id.clone();
        self.clips.push(clip);
        self.validate()?;
        Ok(id)
    }

    pub fn split_clip(&mut self, id: &str, split_at: f64) -> Result<String> {
        let mut candidate = self.clone();
        let selected = candidate
            .clips
            .iter()
            .find(|clip| clip.id == id)
            .context("selected clip no longer exists")?
            .clone();
        let link_id = selected.link_group_id.clone();
        let linked_ids: Vec<_> = candidate
            .clips
            .iter()
            .filter(|clip| {
                clip.id == id
                    || link_id
                        .as_deref()
                        .is_some_and(|link_id| clip.link_group_id.as_deref() == Some(link_id))
            })
            .map(|clip| clip.id.clone())
            .collect();
        if linked_ids.iter().any(|id| {
            candidate
                .clips
                .iter()
                .find(|clip| clip.id == *id)
                .is_some_and(|clip| {
                    candidate
                        .track_for_clip(clip)
                        .is_some_and(|track| track.locked)
                })
        }) {
            bail!("a locked track prevents splitting this clip");
        }
        if candidate.transitions.iter().any(|transition| {
            (linked_ids.contains(&transition.from_clip_id)
                || linked_ids.contains(&transition.to_clip_id))
                && transition.start_time < split_at
                && transition.start_time + transition.duration > split_at
        }) {
            bail!("split point falls inside a transition; adjust or remove the transition first");
        }
        if link_id.is_some() && linked_ids.len() != 2 {
            bail!("linked A/V pair is incomplete; unlink or repair it before editing");
        }
        let right_link = link_id.as_ref().map(|_| unique_id("av-link"));
        let mut selected_right = None;
        for linked_id in linked_ids {
            let new_id = candidate.split_clip_single(&linked_id, split_at)?;
            if linked_id == id {
                selected_right = Some(new_id.clone());
            }
            if let Some(right_link) = &right_link {
                if let Some(right) = candidate.clips.iter_mut().find(|clip| clip.id == new_id) {
                    right.link_group_id = Some(right_link.clone());
                }
            }
        }
        let new_id = selected_right.context("split did not create a selected clip")?;
        candidate.validate()?;
        *self = candidate;
        Ok(new_id)
    }

    fn split_clip_single(&mut self, id: &str, split_at: f64) -> Result<String> {
        let index = self
            .clips
            .iter()
            .position(|clip| clip.id == id)
            .context("selected clip no longer exists")?;
        let original = self.clips[index].clone();
        let relative = split_at - original.timeline_start;
        if relative < 1.0 / f64::from(self.fps)
            || relative > original.duration - 1.0 / f64::from(self.fps)
        {
            bail!("split point must be inside the clip, leaving at least one frame on each side");
        }
        let source_time_mode = original.source_time_mode;
        let playback_rate = original.playback_rate;
        let original_duration = original.duration;
        self.clips[index].duration = relative;
        let mut right = original;
        right.id = unique_id("clip");
        right.name = format!("{} split", right.name);
        for effect in &mut right.effects {
            effect.id = unique_id("effect");
        }
        right.timeline_start = split_at;
        let source_is_unbounded_still = right.kind == ClipKind::Adjustment
            || right
                .source_id
                .as_deref()
                .and_then(|source_id| self.sources.iter().find(|source| source.id == source_id))
                .is_some_and(|source| source.still_image || source.solid_color.is_some());
        if !source_is_unbounded_still {
            match source_time_mode {
                SourceTimeMode::Forward => {
                    right.source_start += relative * playback_rate;
                }
                SourceTimeMode::Reverse => {
                    self.clips[index].source_start +=
                        (original_duration - relative) * playback_rate;
                }
                SourceTimeMode::Freeze => {}
            }
        }
        right.duration -= relative;
        right.position_keyframes.retain(|key| key.time >= split_at);
        if let Some(first) = right.position_keyframes.first_mut() {
            if first.time > split_at {
                first.time = split_at;
            }
        }
        let new_id = right.id.clone();
        self.clips.push(right);
        for transition in &mut self.transitions {
            if transition.start_time >= split_at {
                if transition.from_clip_id == id {
                    transition.from_clip_id = new_id.clone();
                }
                if transition.to_clip_id == id {
                    transition.to_clip_id = new_id.clone();
                }
            }
        }
        Ok(new_id)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        Self::load_with_nested_validation(path.as_ref(), true)
    }

    fn load_with_nested_validation(path: &Path, check_nested_graph: bool) -> Result<Self> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("accessing composition {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > COMPOSITION_FILE_LIMIT {
            bail!(
                "composition must be a regular file no larger than {COMPOSITION_FILE_LIMIT} bytes"
            );
        }
        let mut project: Self = serde_json::from_slice(&fs::read(path)?)?;
        match project.schema_version {
            1 => {
                // Schema 1 used global z-order and a dedicated X-only
                // keyframe list. Preserve both by placing clips on a default
                // compatible track and migrating the keys to the scalar map.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            2 => {
                // Schema 3 adds linked A/V identity, track locks, timeline
                // markers, and generated/still visual source metadata. New
                // fields have defaults, so prior projects remain unchanged.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            3 => {
                // Schema 4 adds an ordered per-clip visual effect stack.
                // Missing stacks default to empty and preserve the legacy
                // clip Gaussian-blur property exactly.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            4 => {
                // Schema 5 adds explicit transitions. Missing transition
                // lists default to empty, preserving the prior timeline.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            5 => {
                // Schema 6 adds constant source-time mapping. Default values
                // preserve forward playback at the historical 1x rate.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            6 => {
                // Schema 7 adds the gamma effect variant. Existing effect
                // stacks need no rewrite; their typed parameters are retained.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            7 => {
                // Schema 8 adds independent user clip groups. Missing IDs
                // default to None, leaving all existing editing semantics intact.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            8 => {
                // Schema 9 adds optional text-box backgrounds. Defaults keep
                // legacy text rendering pixel-identical when boxes are off.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            9 => {
                // Schema 10 adds an optional composition export range. None
                // preserves the historical full-duration export.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            10 => {
                // Schema 11 adds project-local title presets. Missing presets
                // use the built-in local set supplied by CompositionProject::default.
                project.title_presets = default_title_presets();
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            11 => {
                // Schema 12 adds stable composition identity and optional
                // nested-composition references to media sources.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            12 => {
                // Schema 13 adds a project-local H.264/AAC delivery preset.
                // The serde default is Balanced, matching the historical
                // CRF 18 / medium / AAC 192 kbps export exactly.
                project.export_preset = CompositionExportPreset::Balanced;
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            13 => {
                // Schema 14 adds optional generated rectangle/ellipse source
                // geometry. Old generated color sources remain full-frame.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            14 => {
                // Schema 15 adds optional transition easing. Missing values
                // default to linear, preserving prior transition output.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            15 => {
                // Schema 16 adds optional source proxies and the opt-in
                // preview switch. Missing values preserve original-media
                // preview behavior and final exports.
                project.use_proxies_for_preview = false;
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            16 => {
                // Schema 17 records proxy/source signatures and probed output
                // facts. Older path-only proxies remain visible as legacy
                // metadata-missing entries and are never selected silently.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            17 => {
                // Schema 18 adds adjustment layers as a new clip kind without
                // changing the meaning of existing timeline clips.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            18 => {
                // Schema 19 adds optional scalar keyframes to effect instances.
                // Missing maps default to empty, preserving static effect values.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            19 => {
                // Schema 20 adds constant audio-track gain. Missing gain uses
                // unity, preserving the historical mix exactly.
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            20 => {
                // Schema 21 adds optional caption provenance to ordinary text
                // clips and the built-in Short-form caption text preset. Cue
                // text/timing remain in the existing clip fields. Preserve
                // project edits and append only the new missing built-in style.
                for preset in default_title_presets() {
                    if preset.id == "builtin-short-form-caption"
                        && !project
                            .title_presets
                            .iter()
                            .any(|item| item.id == preset.id)
                    {
                        project.title_presets.push(preset);
                    }
                }
                project.schema_version = COMPOSITION_SCHEMA_VERSION;
            }
            COMPOSITION_SCHEMA_VERSION => {}
            version => bail!("unsupported composition schema version {version}"),
        }
        project.normalize_tracks();
        project.validate()?;
        let base = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        for source in &mut project.sources {
            if source.nested_composition_path.is_none()
                && source.solid_color.is_none()
                && source.path.is_relative()
            {
                source.path = base.join(&source.path);
            }
            if let Some(nested_path) = &mut source.nested_composition_path {
                if nested_path.is_relative() {
                    *nested_path = base.join(&*nested_path);
                }
            }
            if let Some(proxy_path) = &mut source.proxy_path {
                if proxy_path.is_relative() {
                    *proxy_path = base.join(&*proxy_path);
                }
            }
        }
        for clip in &mut project.clips {
            if let Some(path) = &mut clip.font_path {
                if path.is_relative() {
                    *path = base.join(&*path);
                }
            }
        }
        if check_nested_graph {
            validate_nested_graph(&project, &mut Vec::new(), &mut HashSet::new())?;
        }
        Ok(project)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let mut normalized = self.clone();
        normalized.schema_version = COMPOSITION_SCHEMA_VERSION;
        normalized.normalize_tracks();
        normalized.validate()?;
        validate_nested_graph(&normalized, &mut Vec::new(), &mut HashSet::new())?;
        let path = path.as_ref();
        let parent = path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("creating composition directory {}", parent.display()))?;
        let mut stored = normalized;
        for source in &mut stored.sources {
            if source.nested_composition_path.is_none() && source.solid_color.is_none() {
                if let Some(relative) = relative_path(parent, &source.path) {
                    source.path = relative;
                }
            }
            if let Some(nested_path) = &mut source.nested_composition_path {
                if let Some(relative) = relative_path(parent, nested_path) {
                    *nested_path = relative;
                }
            }
            if let Some(proxy_path) = &mut source.proxy_path {
                if let Some(relative) = relative_path(parent, proxy_path) {
                    *proxy_path = relative;
                }
            }
        }
        for clip in &mut stored.clips {
            if let Some(path) = &mut clip.font_path {
                if let Some(relative) = relative_path(parent, path) {
                    *path = relative;
                }
            }
        }
        let bytes =
            serde_json::to_vec_pretty(&stored).context("serializing composition project")?;
        if bytes.len() as u64 > COMPOSITION_FILE_LIMIT {
            bail!("composition exceeds the {COMPOSITION_FILE_LIMIT}-byte limit");
        }
        let mut temp = tempfile::NamedTempFile::new_in(parent)
            .context("creating composition save staging file")?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .map_err(|error| error.error)
            .with_context(|| format!("atomically saving composition {}", path.display()))?;
        Ok(())
    }
}

const RECOVERY_FORMAT: &str = "sono-renderer-composition-recovery";
const RECOVERY_SCHEMA_VERSION: u32 = 1;
const MAX_RECOVERY_SNAPSHOTS: usize = 20;

/// Persist an editing checkpoint separately from the user's explicit project
/// save. Checkpoints store absolute media paths so recovery is independent of
/// the application's state-directory location.
pub fn save_recovery_snapshot(
    project: &CompositionProject,
    project_path: Option<&Path>,
) -> Result<PathBuf> {
    let path = recovery_snapshot_path(project_path)?;
    write_recovery_snapshot(&path, project, project_path)?;
    prune_recovery_snapshots(path.parent().context("recovery path has no parent")?)?;
    Ok(path)
}

pub fn write_recovery_snapshot(
    path: &Path,
    project: &CompositionProject,
    project_path: Option<&Path>,
) -> Result<()> {
    let mut project = project.clone();
    project.schema_version = COMPOSITION_SCHEMA_VERSION;
    project.normalize_tracks();
    project.validate()?;
    let current_directory = std::env::current_dir()?;
    for source in &mut project.sources {
        if source.nested_composition_path.is_none()
            && source.solid_color.is_none()
            && source.path.is_relative()
        {
            source.path = current_directory.join(&source.path);
        }
        if let Some(nested_path) = &mut source.nested_composition_path {
            if nested_path.is_relative() {
                *nested_path = current_directory.join(&*nested_path);
            }
        }
        if let Some(proxy_path) = &mut source.proxy_path {
            if proxy_path.is_relative() {
                *proxy_path = current_directory.join(&*proxy_path);
            }
        }
    }
    for clip in &mut project.clips {
        if let Some(path) = &mut clip.font_path {
            if path.is_relative() {
                *path = current_directory.join(&*path);
            }
        }
    }
    let envelope = CompositionRecoveryEnvelope {
        format: RECOVERY_FORMAT.into(),
        schema_version: RECOVERY_SCHEMA_VERSION,
        project_path: project_path.map(Path::to_path_buf),
        checkpoint_unix_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system time precedes Unix epoch")?
            .as_millis(),
        project,
    };
    let bytes = serde_json::to_vec(&envelope).context("serializing composition recovery")?;
    if bytes.len() as u64 > COMPOSITION_FILE_LIMIT {
        bail!("composition recovery exceeds the {COMPOSITION_FILE_LIMIT}-byte limit");
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("composition recovery path has no parent")?;
    fs::create_dir_all(parent)
        .with_context(|| format!("creating recovery directory {}", parent.display()))?;
    let mut staging = tempfile::NamedTempFile::new_in(parent)
        .context("creating composition recovery staging file")?;
    staging.write_all(&bytes)?;
    staging.as_file().sync_all()?;
    staging
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("publishing recovery checkpoint {}", path.display()))?;
    Ok(())
}

pub fn load_recovery_snapshot(path: &Path) -> Result<CompositionRecoverySnapshot> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("reading recovery checkpoint metadata {}", path.display()))?;
    if metadata.len() > COMPOSITION_FILE_LIMIT {
        bail!("composition recovery exceeds the {COMPOSITION_FILE_LIMIT}-byte limit");
    }
    let bytes = fs::read(path)
        .with_context(|| format!("reading recovery checkpoint {}", path.display()))?;
    let mut envelope: CompositionRecoveryEnvelope =
        serde_json::from_slice(&bytes).context("parsing composition recovery checkpoint")?;
    if envelope.format != RECOVERY_FORMAT {
        bail!("file is not a Sono-Renderer composition recovery checkpoint");
    }
    if envelope.schema_version != RECOVERY_SCHEMA_VERSION {
        bail!(
            "unsupported composition recovery schema version {}",
            envelope.schema_version
        );
    }
    if envelope.project.schema_version > COMPOSITION_SCHEMA_VERSION {
        bail!(
            "recovery contains unsupported composition schema version {}",
            envelope.project.schema_version
        );
    }
    envelope.project.normalize_tracks();
    envelope.project.validate()?;
    Ok(CompositionRecoverySnapshot {
        project_path: envelope.project_path,
        checkpoint_unix_ms: envelope.checkpoint_unix_ms,
        project: envelope.project,
    })
}

pub fn list_recovery_snapshots() -> Result<Vec<(PathBuf, CompositionRecoverySnapshot)>> {
    let directory = recovery_directory();
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    for entry in fs::read_dir(&directory)
        .with_context(|| format!("listing recovery checkpoints in {}", directory.display()))?
        .take(128)
        .flatten()
    {
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort_by_key(|path| {
        fs::metadata(path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH)
    });
    let mut snapshots = Vec::new();
    for path in paths.into_iter().rev().take(MAX_RECOVERY_SNAPSHOTS) {
        if let Ok(snapshot) = load_recovery_snapshot(&path) {
            snapshots.push((path, snapshot));
        }
    }
    Ok(snapshots)
}

pub fn remove_recovery_snapshot(project_path: Option<&Path>) -> Result<()> {
    let path = recovery_snapshot_path(project_path)?;
    match fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => {
            Err(error).with_context(|| format!("removing recovery checkpoint {}", path.display()))
        }
    }
}

fn recovery_directory() -> PathBuf {
    crate::store_paths::state_root()
        .join("presentation-studio")
        .join("recovery")
}

fn recovery_snapshot_path(project_path: Option<&Path>) -> Result<PathBuf> {
    let identity = if let Some(path) = project_path {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        crate::content_hash::sha256_hex(absolute.to_string_lossy().replace('\\', "/").as_bytes())
    } else {
        "unsaved-draft".into()
    };
    Ok(recovery_directory().join(format!("{identity}.json")))
}

fn prune_recovery_snapshots(directory: &Path) -> Result<()> {
    let mut entries = fs::read_dir(directory)?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = entry.metadata().ok()?;
            (path
                .extension()
                .is_some_and(|extension| extension == "json"))
            .then(|| (path, metadata.modified().unwrap_or(UNIX_EPOCH)))
        })
        .collect::<Vec<_>>();
    entries.sort_by_key(|(_, modified)| *modified);
    for (path, _) in entries.into_iter().rev().skip(MAX_RECOVERY_SNAPSHOTS) {
        fs::remove_file(&path)
            .with_context(|| format!("pruning old recovery checkpoint {}", path.display()))?;
    }
    Ok(())
}

#[derive(Debug, Clone)]
struct MediaProbe {
    duration: f64,
    has_video: bool,
    has_audio: bool,
    width: Option<u32>,
    height: Option<u32>,
    video_codec: Option<String>,
    still_image: bool,
}

fn media_file_signature(path: &Path) -> Result<MediaFileSignature> {
    let metadata =
        fs::metadata(path).with_context(|| format!("reading media identity {}", path.display()))?;
    if !metadata.is_file() {
        bail!("media identity is not a regular file: {}", path.display());
    }
    let modified_unix_nanos = metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos());
    Ok(MediaFileSignature {
        length: metadata.len(),
        modified_unix_nanos,
    })
}

fn probe_media(path: &Path) -> Result<MediaProbe> {
    let tools = crate::ffmpeg::resolve()?;
    probe_media_with_tools(path, &tools)
}

fn probe_media_with_tools(
    path: &Path,
    tools: &crate::ffmpeg::FfmpegInstallation,
) -> Result<MediaProbe> {
    probe_media_using_tools(path, tools, false)
}

fn probe_media_offline_with_tools(
    path: &Path,
    tools: &crate::ffmpeg::FfmpegInstallation,
) -> Result<MediaProbe> {
    probe_media_using_tools(path, tools, true)
}

fn probe_media_using_tools(
    path: &Path,
    tools: &crate::ffmpeg::FfmpegInstallation,
    local_protocols_only: bool,
) -> Result<MediaProbe> {
    if !path.is_file() {
        bail!("media file does not exist: {}", path.display());
    }
    let mut command = Command::new(&tools.ffprobe);
    command.args(["-v", "error"]);
    if local_protocols_only {
        command.args(["-protocol_whitelist", "file,pipe,crypto,data"]);
    }
    let output = crate::export_control::ExportControl::default()
        .output(crate::export_control::hide_child_window(
        command
            .args([
                "-show_entries",
                "format=format_name,duration:stream=codec_type,codec_name,width,height,duration",
                "-of",
                "json",
            ])
            .arg(path),
    ))?;
    if !output.status.success() {
        bail!(
            "FFprobe could not inspect {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: Value =
        serde_json::from_slice(&output.stdout).context("FFprobe returned invalid JSON")?;
    let streams = value["streams"]
        .as_array()
        .context("media file contains no streams")?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"].as_str() == Some("video"));
    let has_audio = streams
        .iter()
        .any(|stream| stream["codec_type"].as_str() == Some("audio"));
    let format_names = value["format"]["format_name"].as_str().unwrap_or_default();
    let still_image = video.is_some()
        && !has_audio
        && format_names.split(',').any(|name| {
            matches!(
                name,
                "image2"
                    | "image2pipe"
                    | "png_pipe"
                    | "jpeg_pipe"
                    | "bmp_pipe"
                    | "webp_pipe"
                    | "tiff_pipe"
            )
        });
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .or_else(|| {
            streams
                .iter()
                .filter_map(|stream| stream["duration"].as_str()?.parse::<f64>().ok())
                .find(|duration| duration.is_finite() && *duration > 0.0)
        })
        .or_else(|| still_image.then_some(0.0))
        .context("media has no finite positive duration")?;
    if video.is_none() && !has_audio {
        bail!("media has neither an audio nor a video stream");
    }
    Ok(MediaProbe {
        duration,
        has_video: video.is_some(),
        has_audio,
        width: video
            .and_then(|stream| stream["width"].as_u64())
            .and_then(|value| u32::try_from(value).ok()),
        height: video
            .and_then(|stream| stream["height"].as_u64())
            .and_then(|value| u32::try_from(value).ok()),
        video_codec: video
            .and_then(|stream| stream["codec_name"].as_str())
            .map(str::to_owned),
        still_image,
    })
}

/// Probe a local audio source and build its ordinary timeline waveform using
/// only an already-installed FFmpeg. This never installs or downloads tools.
pub(crate) fn inspect_audio_with_existing_tools(path: &Path) -> Result<(f64, Vec<f32>)> {
    let path = path
        .canonicalize()
        .with_context(|| format!("opening audio media {}", path.display()))?;
    let tools = crate::ffmpeg::inspect_existing()?
        .context("tempo analysis needs the application's existing FFmpeg installation")?;
    let probe = probe_media_offline_with_tools(&path, &tools)?;
    if !probe.has_audio || !probe.duration.is_finite() || probe.duration <= 0.0 {
        bail!("selected media does not contain a usable audio stream");
    }
    let waveform =
        crate::media_audio::waveform_for_media_with_tools(&path, probe.duration, 512, &tools)?;
    Ok((probe.duration, waveform))
}

/// Create an editing proxy beside a caller-selected destination. Proxies are
/// video-only and retain source timestamps; preview audio continues to read
/// the original. The destination is published only after probe validation.
pub fn generate_video_proxy(
    source_path: &Path,
    output_path: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<PathBuf> {
    let source_path = source_path
        .canonicalize()
        .with_context(|| format!("resolving proxy source {}", source_path.display()))?;
    let source = probe_media(&source_path)?;
    if !source.has_video || source.still_image {
        bail!("video proxies require an ordinary video source");
    }
    let (source_width, source_height) = source
        .width
        .zip(source.height)
        .context("proxy source has no video dimensions")?;
    let output_path = if output_path.is_absolute() {
        output_path.to_path_buf()
    } else {
        std::env::current_dir()?.join(output_path)
    };
    if !output_path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp4"))
    {
        bail!("proxy output must use the .mp4 extension");
    }
    if output_path.exists() {
        bail!("proxy output already exists; choose another path to avoid overwriting");
    }
    let parent = output_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating proxy output directory {}", parent.display()))?;
    let output_name = output_path
        .file_name()
        .context("proxy output path has no filename")?;
    let resolved_output = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(output_name);
    if resolved_output == source_path {
        bail!("proxy output must not replace its source media");
    }
    let max_edge = 960.0;
    let scale = (max_edge / f64::from(source_width.max(source_height))).min(1.0);
    let width = ((f64::from(source_width) * scale).round() as u32).max(2) & !1;
    let height = ((f64::from(source_height) * scale).round() as u32).max(2) & !1;
    let staging = tempfile::tempdir_in(parent).context("creating proxy staging directory")?;
    let staged_path = staging.path().join("proxy.mp4");
    let tools = crate::ffmpeg::resolve_controlled(control)?;
    let mut command = Command::new(&tools.ffmpeg);
    command
        .args(["-hide_banner", "-loglevel", "error", "-i"])
        .arg(&source_path)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(format!("scale={width}:{height}:flags=lanczos"))
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-crf",
            "28",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
            "-y",
        ])
        .arg(&staged_path);
    let result = control
        .output(crate::export_control::hide_child_window(&mut command))
        .context("running FFmpeg video-proxy encoder")?;
    if !result.status.success() {
        bail!(
            "video-proxy encoding failed: {}",
            String::from_utf8_lossy(&result.stderr)
                .chars()
                .take(4000)
                .collect::<String>()
        );
    }
    control.check()?;
    let proxy = probe_media(&staged_path).context("validating encoded video proxy")?;
    if !proxy.has_video || proxy.has_audio {
        bail!("generated proxy must contain video only");
    }
    if proxy.width != Some(width) || proxy.height != Some(height) {
        bail!("generated proxy dimensions do not match the requested preview size");
    }
    if (proxy.duration - source.duration).abs() > 0.1 {
        bail!("generated proxy duration does not match the source media");
    }
    control.check()?;
    fs::rename(&staged_path, &resolved_output)
        .with_context(|| format!("publishing video proxy {}", resolved_output.display()))?;
    Ok(resolved_output)
}

pub fn render(
    project: &CompositionProject,
    output: impl AsRef<Path>,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    render_inner(project, output.as_ref(), control, None)
}

pub fn preview_png(
    project: &CompositionProject,
    time: f64,
    control: &crate::export_control::ExportControl,
) -> Result<Vec<u8>> {
    finite_range(time, 0.0, project.duration, "preview time")?;
    let temp = tempfile::tempdir().context("creating composition preview workspace")?;
    let path = temp.path().join("preview.png");
    render_inner(project, &path, control, Some(time))?;
    fs::read(path).context("reading rendered composition preview")
}

/// Decode one bounded still from ordinary video media for the media-bin UI.
/// This is a source thumbnail, not a composition preview; callers run it on a
/// background worker and key the cache by the source file identity.
pub(crate) fn media_thumbnail_png(path: &Path, width: u32, height: u32) -> Result<Vec<u8>> {
    if width == 0 || height == 0 || width > 512 || height > 512 {
        bail!("media thumbnail bounds must be in 1..=512 pixels");
    }
    if !path.is_file() {
        bail!("media source is missing: {}", path.display());
    }
    let tools = crate::ffmpeg::resolve()?;
    let scale =
        format!("scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2");
    let mut last_error = String::new();
    for seek in ["0.5", "0"] {
        let output = Command::new(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", seek, "-i"])
            .arg(path)
            .args([
                "-an",
                "-frames:v",
                "1",
                "-vf",
                &scale,
                "-threads",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ])
            .output()
            .with_context(|| format!("starting FFmpeg thumbnail decode for {}", path.display()))?;
        if output.status.success() && !output.stdout.is_empty() {
            image::load_from_memory(&output.stdout)
                .context("FFmpeg emitted an invalid media thumbnail")?;
            return Ok(output.stdout);
        }
        last_error = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1000)
            .collect();
    }
    bail!(
        "FFmpeg could not decode a thumbnail from {}: {last_error}",
        path.display()
    )
}

/// Render one selected visual source in isolation for the editor's source
/// monitor. It uses the same FFmpeg compositor as the program monitor/export,
/// while keeping source time separate from composition time.
pub fn preview_source_png(
    project: &CompositionProject,
    source_id: &str,
    source_time: f64,
    control: &crate::export_control::ExportControl,
) -> Result<Vec<u8>> {
    let source = project
        .sources
        .iter()
        .find(|source| source.id == source_id)
        .context("selected media source no longer exists")?
        .clone();
    if !source.has_video {
        bail!("source monitor preview requires visual media");
    }
    if !source_time.is_finite() || source_time < 0.0 {
        bail!("source monitor time must be finite and nonnegative");
    }
    let frame = 1.0 / f64::from(project.fps);
    let source_time = if source.still_image || source.solid_color.is_some() {
        0.0
    } else {
        source_time.min((source.duration - frame).max(0.0))
    };
    let mut source_project = CompositionProject::new();
    source_project.width = project.width;
    source_project.height = project.height;
    source_project.fps = project.fps;
    source_project.duration = 1.0;
    source_project.sources.push(source.clone());
    let clip_id = source_project.add_clip(&source.id, ClipKind::Video, 0.0)?;
    let clip = source_project
        .clips
        .iter_mut()
        .find(|clip| clip.id == clip_id)
        .context("source monitor clip was not created")?;
    clip.source_start = source_time;
    clip.duration = if source.still_image || source.solid_color.is_some() {
        1.0
    } else {
        (source.duration - source_time).min(1.0).max(frame)
    };
    preview_png(&source_project, 0.0, control)
}

fn render_inner(
    project: &CompositionProject,
    output: &Path,
    control: &crate::export_control::ExportControl,
    preview_time: Option<f64>,
) -> Result<()> {
    project.validate()?;
    control.check()?;
    if project.clips.is_empty() {
        bail!("composition has no clips to render");
    }
    let parent = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating output directory {}", parent.display()))?;
    let temp =
        tempfile::tempdir_in(parent).context("creating composition export staging directory")?;
    let staged = temp.path().join(if preview_time.is_some() {
        "preview.png"
    } else {
        "composition.mp4"
    });
    let tools = crate::ffmpeg::resolve_controlled(control)?;
    let graph_project = materialize_nested_sources(project, temp.path(), control, &mut Vec::new())?;
    let (args, cwd) = build_ffmpeg_args(&graph_project, &staged, temp.path(), preview_time)?;
    let mut command = Command::new(&tools.ffmpeg);
    command.args(args).current_dir(cwd);
    let result = control
        .output(crate::export_control::hide_child_window(&mut command))
        .context("running standalone FFmpeg composition")?;
    if !result.status.success() {
        let detail = String::from_utf8_lossy(&result.stderr);
        bail!(
            "composition export failed: {}",
            detail.chars().take(4000).collect::<String>()
        );
    }
    control.check()?;
    if preview_time.is_some() {
        fs::copy(&staged, output).context("copying composition preview frame")?;
    } else {
        if output.exists() {
            bail!("output already exists; choose a new output path to avoid overwriting it");
        }
        validate_export(
            &tools.ffprobe,
            &staged,
            &graph_project,
            project.export_duration(),
        )?;
        fs::rename(&staged, output)
            .with_context(|| format!("publishing composition output {}", output.display()))?;
    }
    Ok(())
}

fn validate_export(
    ffprobe: &Path,
    path: &Path,
    project: &CompositionProject,
    expected_duration: f64,
) -> Result<()> {
    let output = crate::export_control::ExportControl::default().output(
        crate::export_control::hide_child_window(&mut Command::new(ffprobe))
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration,format_name:stream=codec_type,codec_name,width,height",
                "-of",
                "json",
            ])
            .arg(path),
    )?;
    if !output.status.success() {
        bail!(
            "FFprobe rejected the composition output: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let value: Value = serde_json::from_slice(&output.stdout)?;
    let streams = value["streams"]
        .as_array()
        .context("composition output has no streams")?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"].as_str() == Some("video"))
        .context("composition output has no video stream")?;
    if video["codec_name"].as_str() != Some("h264") {
        bail!("composition output video codec is not H.264");
    }
    if video["width"].as_u64() != Some(u64::from(project.width))
        || video["height"].as_u64() != Some(u64::from(project.height))
    {
        bail!("composition output dimensions do not match the project");
    }
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|text| text.parse::<f64>().ok())
        .context("composition output has no duration")?;
    if !duration.is_finite() || duration <= 0.0 {
        bail!("composition output has an invalid duration");
    }
    if (duration - expected_duration).abs() > (1.0 / f64::from(project.fps)) + 0.05 {
        bail!("composition output duration does not match the selected export range");
    }
    let expects_audio = project
        .clips
        .iter()
        .any(|clip| clip.kind == ClipKind::Audio || clip.include_audio);
    let has_audio = streams
        .iter()
        .any(|stream| stream["codec_type"].as_str() == Some("audio"));
    if expects_audio != has_audio {
        bail!("composition output audio streams do not match the project");
    }
    if let Some(format) = value["format"]["format_name"].as_str() {
        if !format.split(',').any(|name| name == "mp4") {
            bail!("composition output container is not MP4");
        }
    } else {
        bail!("composition output has no container metadata");
    }
    Ok(())
}

fn materialize_nested_sources(
    project: &CompositionProject,
    work: &Path,
    control: &crate::export_control::ExportControl,
    ancestors: &mut Vec<String>,
) -> Result<CompositionProject> {
    if ancestors.iter().any(|id| id == &project.composition_id) {
        bail!("nested composition cycle detected at {}", project.name);
    }
    if ancestors.len() >= 16 {
        bail!("nested compositions may not exceed 16 levels");
    }
    control.check()?;
    ancestors.push(project.composition_id.clone());
    let mut materialized = project.clone();
    for (index, source) in materialized.sources.iter_mut().enumerate() {
        let Some(child_path) = source.nested_composition_path.clone() else {
            continue;
        };
        if !child_path.is_file() {
            bail!(
                "missing nested composition {:?}: {}",
                source.display_name,
                child_path.display()
            );
        }
        let child = CompositionProject::load_with_nested_validation(&child_path, false)
            .with_context(|| format!("loading nested composition {}", child_path.display()))?;
        let child = materialize_nested_sources(&child, work, control, ancestors)?;
        let output = work.join(format!(
            "nested-{:016x}-{index}.mkv",
            stable_path_token(&project.composition_id)
        ));
        if !output.is_file() || fs::metadata(&output).map(|meta| meta.len()).unwrap_or(0) == 0 {
            render_nested_intermediate(&child, &output, work, control)?;
        }
        source.path = output;
        source.nested_composition_path = None;
        source.has_video = true;
        source.has_audio = child.routes_audio();
        source.duration = child.duration;
        source.width = Some(child.width);
        source.height = Some(child.height);
        source.still_image = false;
        source.solid_color = None;
        source.waveform_peaks.clear();
    }
    ancestors.pop();
    materialized.validate()?;
    Ok(materialized)
}

fn render_nested_intermediate(
    project: &CompositionProject,
    output: &Path,
    work: &Path,
    control: &crate::export_control::ExportControl,
) -> Result<()> {
    control.check()?;
    let tools = crate::ffmpeg::resolve_controlled(control)?;
    let name = format!(
        "nested-work-{:016x}",
        stable_path_token(&project.composition_id)
    );
    let filter_work = work.join(name);
    fs::create_dir_all(&filter_work).context("creating nested-composition filter workspace")?;
    let mut full_duration = project.clone();
    full_duration.export_range = None;
    let (args, cwd) =
        build_ffmpeg_args_mode(&full_duration, output, &filter_work, None, false, true)?;
    let mut command = Command::new(&tools.ffmpeg);
    command.args(args).current_dir(cwd);
    let result = control
        .output(crate::export_control::hide_child_window(&mut command))
        .context("rendering nested composition intermediate")?;
    if !result.status.success() {
        bail!(
            "nested composition render failed: {}",
            String::from_utf8_lossy(&result.stderr)
                .chars()
                .take(4000)
                .collect::<String>()
        );
    }
    if !output.is_file() || fs::metadata(output)?.len() == 0 {
        bail!("nested composition render produced no media");
    }
    let probe = crate::export_control::ExportControl::default().output(
        crate::export_control::hide_child_window(&mut Command::new(&tools.ffprobe))
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type,codec_name,width,height",
                "-of",
                "json",
            ])
            .arg(output),
    )?;
    if !probe.status.success() {
        bail!("FFprobe rejected the nested composition intermediate");
    }
    let value: Value = serde_json::from_slice(&probe.stdout)?;
    let video = value["streams"]
        .as_array()
        .and_then(|streams| {
            streams
                .iter()
                .find(|stream| stream["codec_type"] == "video")
        })
        .context("nested composition intermediate has no video stream")?;
    if video["codec_name"] != "ffv1"
        || video["width"].as_u64() != Some(u64::from(project.width))
        || video["height"].as_u64() != Some(u64::from(project.height))
    {
        bail!("nested composition intermediate is not full-resolution FFV1");
    }
    let duration = value["format"]["duration"]
        .as_str()
        .and_then(|duration| duration.parse::<f64>().ok())
        .context("nested composition intermediate has no duration")?;
    if (duration - project.duration).abs() > (1.0 / f64::from(project.fps)) + 0.05 {
        bail!("nested composition intermediate duration is incorrect");
    }
    let has_audio = value["streams"]
        .as_array()
        .is_some_and(|streams| streams.iter().any(|stream| stream["codec_type"] == "audio"));
    if has_audio != project.routes_audio() {
        bail!("nested composition intermediate audio routing is incorrect");
    }
    Ok(())
}

fn stable_path_token(value: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

fn build_ffmpeg_args(
    project: &CompositionProject,
    output: &Path,
    work: &Path,
    preview_time: Option<f64>,
) -> Result<(Vec<std::ffi::OsString>, PathBuf)> {
    build_ffmpeg_args_mode(project, output, work, preview_time, false, false)
}

fn build_ffmpeg_args_mode(
    project: &CompositionProject,
    output: &Path,
    work: &Path,
    preview_time: Option<f64>,
    video_pipe: bool,
    lossless_intermediate: bool,
) -> Result<(Vec<std::ffi::OsString>, PathBuf)> {
    project.validate()?;
    let source_map: HashMap<&str, &MediaSource> = project
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source))
        .collect();
    let active_sources: Vec<_> = project
        .sources
        .iter()
        .filter(|source| {
            project
                .clips
                .iter()
                .any(|clip| clip.source_id.as_deref() == Some(source.id.as_str()))
        })
        .collect();
    let input_sources: Vec<_> = active_sources
        .iter()
        .copied()
        .filter(|source| source.solid_color.is_none())
        .collect();
    let mut args = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
    ];
    let use_proxies = project.use_proxies_for_preview && (preview_time.is_some() || video_pipe);
    for source in &input_sources {
        {
            let input_path = if use_proxies {
                let proxy_is_current = matches!(
                    project.source_proxy_status(&source.id),
                    Some(VideoProxyStatus::Ready | VideoProxyStatus::ReadyOriginalOffline)
                );
                if proxy_is_current {
                    source
                        .proxy_path
                        .as_deref()
                        .unwrap_or(source.path.as_path())
                } else {
                    source.path.as_path()
                }
            } else {
                source.path.as_path()
            };
            if !input_path.is_file() {
                bail!(
                    "missing composition source {:?}: {}",
                    source.display_name,
                    input_path.display()
                );
            }
            if source.still_image {
                args.extend(["-loop".into(), "1".into(), "-framerate".into()]);
                args.push(project.fps.to_string().into());
            }
            args.push("-i".into());
            args.push(input_path.as_os_str().to_owned());
        }
    }
    let indexes: HashMap<&str, usize> = active_sources
        .iter()
        .filter(|source| source.solid_color.is_none())
        .enumerate()
        .map(|(i, source)| (source.id.as_str(), i))
        .collect();
    let mut graph = Vec::<String>::new();
    graph.push(format!(
        "color=c=black:s={}x{}:r={}:d={:.6}[base0]",
        project.width, project.height, project.fps, project.duration
    ));
    let ordered: Vec<_> = {
        let mut clips: Vec<_> = project.clips.iter().collect();
        clips.sort_by(|a, b| {
            let a_track = project.track_for_clip(a);
            let b_track = project.track_for_clip(b);
            let a_order = a_track.map_or(0, |track| track.order);
            let b_order = b_track.map_or(0, |track| track.order);
            a_order
                .cmp(&b_order)
                .then_with(|| a.z_order.cmp(&b.z_order))
                .then_with(|| a.timeline_start.total_cmp(&b.timeline_start))
        });
        clips
    };
    let mut consumers: HashMap<&str, Vec<&CompositionClip>> = HashMap::new();
    for clip in &ordered {
        if let Some(id) = clip.source_id.as_deref() {
            consumers.entry(id).or_default().push(clip);
        }
    }
    let mut split_labels = HashMap::<String, Vec<String>>::new();
    for source in &active_sources {
        if source.solid_color.is_some() {
            continue;
        }
        let clips = consumers
            .get(source.id.as_str())
            .cloned()
            .unwrap_or_default();
        let index = indexes[source.id.as_str()];
        let video_clips: Vec<_> = clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Video)
            .collect();
        if !video_clips.is_empty() && source.has_video {
            let input_label = if use_proxies
                && matches!(
                    project.source_proxy_status(&source.id),
                    Some(VideoProxyStatus::Ready | VideoProxyStatus::ReadyOriginalOffline)
                ) {
                let width = source
                    .width
                    .context("proxy source has no canonical width")?;
                let height = source
                    .height
                    .context("proxy source has no canonical height")?;
                let label = format!("vproxy_norm_{index}");
                // Preserve the source-coordinate model used by crop, scale,
                // rotation, and keyframes. The proxy is decoded smaller, then
                // normalized before branching into clip instances.
                graph.push(format!(
                    "[{index}:v]scale={width}:{height}:flags=bilinear,setsar=1[{label}]"
                ));
                format!("[{label}]")
            } else {
                format!("[{index}:v]")
            };
            let outputs: Vec<_> = (0..video_clips.len())
                .map(|n| format!("vsrc_{index}_{n}"))
                .collect();
            let filter = if outputs.len() == 1 {
                format!("{input_label}null[{}]", outputs[0])
            } else {
                format!(
                    "{input_label}split={}[{}]",
                    outputs.len(),
                    outputs.join("][")
                )
            };
            graph.push(filter);
            split_labels.insert(format!("{}:v", source.id), outputs);
        }
        let audio_clips: Vec<_> = clips
            .iter()
            .filter(|clip| {
                preview_time.is_none()
                    && !video_pipe
                    && (clip.kind == ClipKind::Audio || clip.include_audio)
            })
            .collect();
        if !audio_clips.is_empty() && source.has_audio {
            let outputs: Vec<_> = (0..audio_clips.len())
                .map(|n| format!("asrc_{index}_{n}"))
                .collect();
            let filter = if outputs.len() == 1 {
                format!("[{index}:a]anull[{}]", outputs[0])
            } else {
                format!(
                    "[{index}:a]asplit={}[{}]",
                    outputs.len(),
                    outputs.join("][")
                )
            };
            graph.push(filter);
            split_labels.insert(format!("{}:a", source.id), outputs);
        }
    }
    let mut video_index = HashMap::<String, usize>::new();
    let mut audio_index = HashMap::<String, usize>::new();
    let audio_solo_active = project
        .clips
        .iter()
        .any(|clip| clip.kind == ClipKind::Audio && clip.solo);
    let audio_track_solo_active = project
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Audio && track.solo);
    let mut base = "base0".to_owned();
    let mut audio_labels = Vec::new();
    for (clip_index, clip) in ordered.iter().enumerate() {
        match clip.kind {
            ClipKind::Video => {
                let source_id = clip.source_id.as_deref().unwrap();
                let source = source_map[source_id];
                let input_label = if let Some(color) = source.solid_color.as_deref() {
                    let label = format!("solid_{clip_index}");
                    let width = source.width.unwrap_or(project.width);
                    let height = source.height.unwrap_or(project.height);
                    if let Some(shape) = source.generated_shape {
                        let x = shape.x * f64::from(width);
                        let y = shape.y * f64::from(height);
                        let shape_width = shape.width * f64::from(width);
                        let shape_height = shape.height * f64::from(height);
                        let alpha = match shape.kind {
                            GeneratedShapeKind::Rectangle | GeneratedShapeKind::Line => format!(
                                "if(gte(X,{x:.4})*lt(X,{:.4})*gte(Y,{y:.4})*lt(Y,{:.4}),255,0)",
                                x + shape_width,
                                y + shape_height,
                            ),
                            GeneratedShapeKind::Ellipse => format!(
                                "if(lte(pow((X-{:.4})/{:.4},2)+pow((Y-{:.4})/{:.4},2),1),255,0)",
                                x + shape_width / 2.0,
                                shape_width / 2.0,
                                y + shape_height / 2.0,
                                shape_height / 2.0,
                            ),
                        };
                        graph.push(format!(
                            "color=c=0x{color}:s={width}x{height}:r={}:d={:.6},format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='{alpha}'[{label}]",
                            project.fps, project.duration
                        ));
                    } else {
                        graph.push(format!(
                            "color=c=0x{color}:s={width}x{height}:r={}:d={:.6}[{label}]",
                            project.fps, project.duration
                        ));
                    }
                    label
                } else {
                    let branch = *video_index
                        .entry(source_id.to_owned())
                        .and_modify(|n| *n += 1)
                        .or_insert(0);
                    split_labels[&format!("{source_id}:v")][branch].clone()
                };
                let audio_branch = if preview_time.is_none()
                    && !video_pipe
                    && clip.include_audio
                    && source.has_audio
                {
                    Some(
                        *audio_index
                            .entry(source_id.to_owned())
                            .and_modify(|n| *n += 1)
                            .or_insert(0),
                    )
                } else {
                    None
                };
                let track = project.track_for_clip(clip);
                if !clip.enabled || track.is_some_and(|track| !track.enabled) {
                    continue;
                }
                let out_label = format!("vclip_{clip_index}");
                let scale_x = property_expr(clip, AnimatedProperty::ScaleX);
                let scale_y = property_expr(clip, AnimatedProperty::ScaleY);
                let rotation = property_expr(clip, AnimatedProperty::Rotation);
                let source_width = source.width.context("video source has no width")?;
                let source_height = source.height.context("video source has no height")?;
                let (crop_commands, blur_commands) = write_video_animation_commands(
                    work,
                    clip,
                    clip_index,
                    source_width,
                    source_height,
                    project.fps,
                )?;
                let start_time = clip.timeline_start;
                let crop_time_values = [
                    clip.animated_value(AnimatedProperty::CropLeft, start_time),
                    clip.animated_value(AnimatedProperty::CropTop, start_time),
                    clip.animated_value(AnimatedProperty::CropRight, start_time),
                    clip.animated_value(AnimatedProperty::CropBottom, start_time),
                ];
                let (crop_width, crop_height, crop_x, crop_y) = crop_geometry(
                    source_width,
                    source_height,
                    crop_time_values[0],
                    crop_time_values[1],
                    crop_time_values[2],
                    crop_time_values[3],
                )?;
                let blur = clip.animated_value(AnimatedProperty::Blur, start_time);
                let mut video_chain = match clip.source_time_mode {
                    SourceTimeMode::Forward if (clip.playback_rate - 1.0).abs() < f64::EPSILON => {
                        format!(
                            "[{input_label}]trim=start={:.6}:duration={:.6},setpts=PTS-STARTPTS,fps={},setpts=PTS+{:.6}/TB",
                            clip.source_start, clip.duration, project.fps, clip.timeline_start
                        )
                    }
                    SourceTimeMode::Forward => format!(
                        "[{input_label}]trim=start={:.6}:duration={:.6},setpts=(PTS-STARTPTS)/{:.6},fps={},setpts=PTS+{:.6}/TB",
                        clip.source_start,
                        clip.duration * clip.playback_rate,
                        clip.playback_rate,
                        project.fps,
                        clip.timeline_start
                    ),
                    SourceTimeMode::Reverse => format!(
                        "[{input_label}]trim=start={:.6}:duration={:.6},setpts=PTS-STARTPTS,reverse,setpts=(PTS-STARTPTS)/{:.6},fps={},setpts=PTS+{:.6}/TB",
                        clip.source_start,
                        clip.duration * clip.playback_rate,
                        clip.playback_rate,
                        project.fps,
                        clip.timeline_start
                    ),
                    SourceTimeMode::Freeze => format!(
                        "[{input_label}]trim=start={:.6}:duration={:.6},setpts=PTS-STARTPTS,tpad=stop_mode=clone:stop_duration={:.6},trim=duration={:.6},fps={},setpts=PTS+{:.6}/TB",
                        clip.source_start,
                        1.0 / f64::from(project.fps),
                        clip.duration,
                        clip.duration,
                        project.fps,
                        clip.timeline_start
                    ),
                };
                if let Some(path) = crop_commands {
                    video_chain.push_str(&format!(",sendcmd=f='{}'", escape_filter_path(&path)));
                }
                video_chain.push_str(&format!(
                    ",crop@crop_{clip_index}=w={crop_width}:h={crop_height}:x={crop_x}:y={crop_y},scale=w='max(2,trunc(iw*({scale_x})/2)*2)':h='max(2,trunc(ih*({scale_y})/2)*2)':eval=frame:flags=lanczos"
                ));
                if let Some(path) = blur_commands {
                    video_chain.push_str(&format!(",sendcmd=f='{}'", escape_filter_path(&path)));
                }
                video_chain.push_str(&format!(
                    ",gblur@blur_{clip_index}=sigma={blur:.4},format=rgba"
                ));
                for effect in &clip.effects {
                    if !effect.enabled {
                        continue;
                    }
                    let effect_filter = visual_effect_filter(effect);
                    video_chain.push_str(&format!(",{effect_filter}"));
                }
                if let Some(mask) = &clip.mask {
                    let mask_x = property_expr_at(clip, AnimatedProperty::MaskX, "T");
                    let mask_y = property_expr_at(clip, AnimatedProperty::MaskY, "T");
                    let mask_width = property_expr_at(clip, AnimatedProperty::MaskWidth, "T");
                    let mask_height = property_expr_at(clip, AnimatedProperty::MaskHeight, "T");
                    let feather = property_expr_at(clip, AnimatedProperty::MaskFeather, "T");
                    let distance = match mask.shape {
                        VisualMaskShape::Rectangle => format!(
                            "min(min(X-({mask_x})*W,(({mask_x})+({mask_width}))*W-X),min(Y-({mask_y})*H,(({mask_y})+({mask_height}))*H-Y))"
                        ),
                        VisualMaskShape::Ellipse => format!(
                            "(1-sqrt(pow((X/W-(({mask_x})+({mask_width})/2))*2/({mask_width}),2)+pow((Y/H-(({mask_y})+({mask_height})/2))*2/({mask_height}),2)))*min(({mask_width})*W,({mask_height})*H)/2"
                        ),
                    };
                    let inside = format!("if(gte({distance},0),1,0)");
                    let feathered =
                        format!("clip(0.5+({distance})/max(0.000001,({feather})*min(W,H)),0,1)");
                    let mut coverage = format!("if(lte(({feather}),0),{inside},{feathered})");
                    if mask.inverted {
                        coverage = format!("1-({coverage})");
                    }
                    video_chain.push_str(&format!(
                        ",geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({coverage})'"
                    ));
                }
                video_chain.push_str(&format!("[vclip_{clip_index}_base]"));
                graph.push(video_chain);
                let mut video_input = format!("vclip_{clip_index}_base");
                if clip.rotation_degrees.abs() > f64::EPSILON
                    || clip.has_animation(AnimatedProperty::Rotation)
                {
                    let scaled_width = f64::from(source_width)
                        * max_animated_value(clip, AnimatedProperty::ScaleX);
                    let scaled_height = f64::from(source_height)
                        * max_animated_value(clip, AnimatedProperty::ScaleY);
                    let bound = even_dimension(scaled_width.hypot(scaled_height));
                    graph.push(format!("[{video_input}]rotate=a='({rotation})*PI/180':ow={bound}:oh={bound}:c=none[vclip_{clip_index}_rot]"));
                    video_input = format!("vclip_{clip_index}_rot");
                }
                let transition_opacity = transition_opacity_expr(project, clip);
                if clip.has_animation(AnimatedProperty::Opacity) || transition_opacity.is_some() {
                    let opacity = property_expr_at(clip, AnimatedProperty::Opacity, "T");
                    let opacity = transition_opacity.map_or(opacity.clone(), |transition| {
                        format!("({opacity})*({transition})")
                    });
                    graph.push(format!(
                        "[{video_input}]geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({opacity})'[{out_label}]"
                    ));
                } else {
                    let opacity = property_expr(clip, AnimatedProperty::Opacity);
                    graph.push(format!(
                        "[{video_input}]colorchannelmixer=aa={opacity}[{out_label}]"
                    ));
                }
                let next = format!("base_{}", clip_index + 1);
                let x = property_expr(clip, AnimatedProperty::X);
                let y = property_expr(clip, AnimatedProperty::Y);
                graph.push(format!("[{base}][{out_label}]overlay=x='{x}':y='{y}':eof_action=pass:shortest=0:repeatlast=0:enable='between(t,{:.6},{:.6})'[{next}]", clip.timeline_start, clip.timeline_start + clip.duration));
                base = next;
                if let Some(branch) = audio_branch {
                    let input_label = split_labels[&format!("{source_id}:a")][branch].clone();
                    audio_labels.push(make_audio_filter(
                        &mut graph,
                        &input_label,
                        clip,
                        clip_index,
                        track.map_or(1.0, |track| track.gain),
                        clip.muted
                            || track.is_some_and(|track| track.muted)
                            || (audio_solo_active && !clip.solo)
                            || (audio_track_solo_active && !track.is_some_and(|track| track.solo)),
                    ));
                }
            }
            ClipKind::Audio => {
                if preview_time.is_some() || video_pipe {
                    continue;
                }
                let source_id = clip.source_id.as_deref().unwrap();
                let branch = *audio_index
                    .entry(source_id.to_owned())
                    .and_modify(|n| *n += 1)
                    .or_insert(0);
                let input_label = split_labels[&format!("{source_id}:a")][branch].clone();
                let track = project.track_for_clip(clip);
                if !clip.enabled || track.is_some_and(|track| !track.enabled) {
                    continue;
                }
                audio_labels.push(make_audio_filter(
                    &mut graph,
                    &input_label,
                    clip,
                    clip_index,
                    track.map_or(1.0, |track| track.gain),
                    clip.muted
                        || track.is_some_and(|track| track.muted)
                        || (audio_solo_active && !clip.solo)
                        || (audio_track_solo_active && !track.is_some_and(|track| track.solo)),
                ));
            }
            ClipKind::Adjustment => {
                let track = project.track_for_clip(clip);
                if !clip.enabled || track.is_some_and(|track| !track.enabled) {
                    continue;
                }
                let enabled_effects: Vec<_> = clip
                    .effects
                    .iter()
                    .filter(|effect| effect.enabled)
                    .collect();
                if enabled_effects.is_empty() {
                    continue;
                }

                // Adjustment layers fork the already-composited image at this
                // z position, process one branch in stack order, then blend it
                // back over the untouched branch for the clip's active range.
                let under = format!("adjust_under_{clip_index}");
                let mut adjusted = format!("adjust_source_{clip_index}");
                graph.push(format!("[{base}]split[{under}][{adjusted}]"));
                for (effect_index, effect) in enabled_effects.iter().enumerate() {
                    let next = format!("adjust_fx_{clip_index}_{effect_index}");
                    graph.push(format!(
                        "[{adjusted}]{}[{next}]",
                        visual_effect_filter(effect)
                    ));
                    adjusted = next;
                }
                let opacity = property_expr_at(clip, AnimatedProperty::Opacity, "T");
                let adjusted_rgba = format!("adjust_rgba_{clip_index}");
                graph.push(format!(
                    "[{adjusted}]format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({opacity})'[{adjusted_rgba}]"
                ));
                let next = format!("base_{}", clip_index + 1);
                graph.push(format!(
                    "[{under}][{adjusted_rgba}]overlay=eof_action=pass:shortest=0:repeatlast=0:enable='between(t,{:.6},{:.6})'[{next}]",
                    clip.timeline_start,
                    clip.timeline_start + clip.duration
                ));
                base = next;
            }
            ClipKind::Text => {
                let track = project.track_for_clip(clip);
                if !clip.enabled || track.is_some_and(|track| !track.enabled) {
                    continue;
                }
                let text_file = work.join(format!("caption_{clip_index}.txt"));
                fs::write(&text_file, clip.text.as_bytes())
                    .context("writing temporary caption text")?;
                let font = if let Some(font) = &clip.font_path {
                    if !font.is_file() {
                        bail!("selected text font is missing: {}", font.display());
                    }
                    font.clone()
                } else {
                    default_font().context("no supported system font was found for text layers")?
                };
                let out = format!("base_{}", clip_index + 1);
                let font = escape_filter_path(&font);
                let text_file = escape_filter_path(&text_file);
                let x = property_expr(clip, AnimatedProperty::X);
                let y = property_expr(clip, AnimatedProperty::Y);
                let x = match clip.text_alignment {
                    TextAlignment::Left => x,
                    TextAlignment::Center => format!("({x})-text_w/2"),
                    TextAlignment::Right => format!("({x})-text_w"),
                };
                let font_size = property_expr(clip, AnimatedProperty::FontSize);
                let opacity = property_expr(clip, AnimatedProperty::Opacity);
                let background = if clip.text_background_enabled {
                    format!(
                        ":box=1:boxcolor=0x{}@{:.3}:boxborderw={}",
                        clip.text_background_color,
                        clip.text_background_opacity,
                        clip.text_background_padding
                    )
                } else {
                    String::new()
                };
                graph.push(format!("[{base}]drawtext=fontfile='{font}':textfile='{text_file}':expansion=none:fontsize='{font_size}':alpha='{opacity}':fontcolor=0x{}:borderw={:.3}:bordercolor=0x{}:shadowx={:.3}:shadowy={:.3}{background}:x='{x}':y='{y}':enable='between(t,{:.6},{:.6})'[{out}]", clip.text_color, clip.outline_width, clip.outline_color, clip.shadow_x, clip.shadow_y, clip.timeline_start, clip.timeline_start + clip.duration));
                base = out;
            }
        }
    }
    let mut final_video = base;
    for (index, transition) in project
        .transitions
        .iter()
        .enumerate()
        .filter(|(_, transition)| {
            matches!(
                transition.kind,
                TransitionKind::DipToBlack | TransitionKind::DipToWhite
            )
        })
    {
        let from = project
            .clips
            .iter()
            .find(|clip| clip.id == transition.from_clip_id);
        let to = project
            .clips
            .iter()
            .find(|clip| clip.id == transition.to_clip_id);
        let active = [from, to].into_iter().flatten().all(|clip| {
            clip.enabled
                && !project
                    .track_for_clip(clip)
                    .is_some_and(|track| !track.enabled)
        });
        if !active {
            continue;
        }
        let color = match transition.kind {
            TransitionKind::DipToBlack => "black",
            TransitionKind::DipToWhite => "white",
            TransitionKind::CrossDissolve => unreachable!(),
        };
        let overlay = format!("dip_overlay_{index}");
        let next = format!("dip_base_{index}");
        let half_duration = transition.duration / 2.0;
        let rising = transition_progress_expr(
            transition.easing,
            format!(
                "clip((T-{:.6})/{half_duration:.6},0,1)",
                transition.start_time
            ),
        );
        let falling = transition_progress_expr(
            transition.easing,
            format!(
                "clip(({:.6}-T)/{half_duration:.6},0,1)",
                transition.start_time + transition.duration
            ),
        );
        let alpha = format!("255*min(({rising}),({falling}))");
        graph.push(format!(
            "color=c={color}:s={}x{}:r={}:d={:.6},format=rgba,geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='{alpha}'[{overlay}]",
            project.width, project.height, project.fps, project.duration
        ));
        graph.push(format!(
            "[{final_video}][{overlay}]overlay=x=0:y=0:eof_action=pass:shortest=0:repeatlast=0:enable='between(t,{:.6},{:.6})'[{next}]",
            transition.start_time,
            transition.start_time + transition.duration
        ));
        final_video = next;
    }
    if !video_pipe && preview_time.is_none() {
        if let Some(range) = project.export_range {
            let out = "export_range_video";
            graph.push(format!(
                "[{final_video}]trim=start={:.6}:end={:.6},setpts=PTS-STARTPTS[{out}]",
                range.start, range.end
            ));
            final_video = out.into();
        }
    }
    if let Some(time) = preview_time {
        let out = "preview_frame";
        let frame_count = (project.duration * f64::from(project.fps)).ceil() as u64;
        let frame =
            ((time * f64::from(project.fps)).round() as u64).min(frame_count.saturating_sub(1));
        graph.push(format!(
            "[{final_video}]select='eq(n,{frame})',setpts=PTS-STARTPTS,scale={}:{}:flags=lanczos[{out}]",
            project.width, project.height
        ));
        final_video = out.into();
    }
    if video_pipe {
        let (width, height) = playback_dimensions(project.width, project.height);
        graph.push(format!(
            "[{final_video}]scale={width}:{height}:flags=fast_bilinear,format=rgba[playback_video]"
        ));
        final_video = "playback_video".into();
    }
    let mut final_audio = None;
    if !audio_labels.is_empty() && preview_time.is_none() && !video_pipe {
        let out = "audio_mix";
        if let Some(range) = project.export_range {
            graph.push(format!("{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0,atrim=start={:.6}:end={:.6},asetpts=PTS-STARTPTS[{out}]", audio_labels.iter().map(|label| format!("[{label}]")).collect::<String>(), audio_labels.len(), range.start, range.end));
        } else {
            graph.push(format!("{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0,atrim=duration={:.6},asetpts=PTS-STARTPTS[{out}]", audio_labels.iter().map(|label| format!("[{label}]")).collect::<String>(), audio_labels.len(), project.duration));
        }
        final_audio = Some(out.to_owned());
    }
    args.push("-filter_complex".into());
    args.push(graph.join(";").into());
    args.push("-map".into());
    args.push(format!("[{final_video}]").into());
    if let Some(audio) = final_audio {
        args.push("-map".into());
        args.push(format!("[{audio}]").into());
        let (_, _, audio_bitrate) = project.export_preset.encoding();
        args.extend([
            "-c:a".into(),
            "aac".into(),
            "-b:a".into(),
            audio_bitrate.into(),
        ]);
    } else if preview_time.is_none() && !video_pipe {
        args.push("-an".into());
    }
    if video_pipe {
        let frame_count = (project.duration * f64::from(project.fps)).ceil() as u64;
        args.extend([
            "-an".into(),
            "-frames:v".into(),
            frame_count.to_string().into(),
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgba".into(),
            "-threads".into(),
            "1".into(),
        ]);
    } else if preview_time.is_some() {
        args.extend(["-frames:v".into(), "1".into(), "-f".into(), "image2".into()]);
    } else if lossless_intermediate {
        args.extend([
            "-t".into(),
            fmt(project.duration).into(),
            "-r".into(),
            project.fps.to_string().into(),
            "-c:v".into(),
            "ffv1".into(),
            "-level".into(),
            "3".into(),
            "-pix_fmt".into(),
            "bgra".into(),
            "-c:a".into(),
            "pcm_s16le".into(),
            "-f".into(),
            "matroska".into(),
        ]);
    } else {
        let (crf, preset, _) = project.export_preset.encoding();
        args.extend([
            "-t".into(),
            fmt(project.export_duration()).into(),
            "-r".into(),
            project.fps.to_string().into(),
            "-c:v".into(),
            "libx264".into(),
            "-pix_fmt".into(),
            "yuv420p".into(),
            "-crf".into(),
            crf.to_string().into(),
            "-preset".into(),
            preset.into(),
            "-movflags".into(),
            "+faststart".into(),
        ]);
    }
    args.push("-y".into());
    args.push(output.as_os_str().to_owned());
    Ok((args, work.to_path_buf()))
}

pub(crate) fn playback_dimensions(width: u32, height: u32) -> (u32, u32) {
    const MAX_EDGE: f64 = 960.0;
    let scale = (MAX_EDGE / f64::from(width.max(height))).min(1.0);
    let width = ((f64::from(width) * scale).floor() as u32).max(2);
    let height = ((f64::from(height) * scale).floor() as u32).max(2);
    (width - width % 2, height - height % 2)
}

pub(crate) fn playback_video_command(
    project: &CompositionProject,
    work: &Path,
) -> Result<(PathBuf, Vec<std::ffi::OsString>, u32, u32)> {
    project.validate()?;
    let tools = crate::ffmpeg::resolve()?;
    let graph_project = materialize_nested_sources(
        project,
        work,
        &crate::export_control::ExportControl::default(),
        &mut Vec::new(),
    )?;
    let (args, _) =
        build_ffmpeg_args_mode(&graph_project, Path::new("-"), work, None, true, false)?;
    let (width, height) = playback_dimensions(project.width, project.height);
    Ok((tools.ffmpeg, args, width, height))
}

pub(crate) fn playback_audio_command(
    project: &CompositionProject,
    sample_rate: u32,
    work: &Path,
) -> Result<(PathBuf, Vec<std::ffi::OsString>)> {
    project.validate()?;
    let graph_project = materialize_nested_sources(
        project,
        work,
        &crate::export_control::ExportControl::default(),
        &mut Vec::new(),
    )?;
    let project = &graph_project;
    let tools = crate::ffmpeg::resolve()?;
    let mut active_sources = Vec::<&MediaSource>::new();
    for clip in &project.clips {
        if !clip.enabled || !matches!(clip.kind, ClipKind::Audio | ClipKind::Video) {
            continue;
        }
        let wants_audio = clip.kind == ClipKind::Audio || clip.include_audio;
        if !wants_audio
            || project
                .track_for_clip(clip)
                .is_some_and(|track| !track.enabled)
        {
            continue;
        }
        let Some(source_id) = clip.source_id.as_deref() else {
            continue;
        };
        let source = project
            .sources
            .iter()
            .find(|source| source.id == source_id)
            .context("audio clip source no longer exists")?;
        if !source.has_audio {
            continue;
        }
        if !source.path.is_file() {
            bail!(
                "missing composition source {:?}: {}",
                source.display_name,
                source.path.display()
            );
        }
        if !active_sources.iter().any(|item| item.id == source_id) {
            active_sources.push(source);
        }
    }
    let source_indexes: HashMap<&str, usize> = active_sources
        .iter()
        .enumerate()
        .map(|(index, source)| (source.id.as_str(), index))
        .collect();
    let mut args = vec!["-hide_banner".into(), "-loglevel".into(), "error".into()];
    for source in &active_sources {
        args.push("-i".into());
        args.push(source.path.as_os_str().to_owned());
    }

    let mut ordered: Vec<_> = project.clips.iter().collect();
    ordered.sort_by(|a, b| a.timeline_start.total_cmp(&b.timeline_start));
    let mut consumers = HashMap::<&str, Vec<&CompositionClip>>::new();
    for clip in &ordered {
        if !clip.enabled || !matches!(clip.kind, ClipKind::Audio | ClipKind::Video) {
            continue;
        }
        if clip.kind == ClipKind::Video && !clip.include_audio {
            continue;
        }
        if project
            .track_for_clip(clip)
            .is_some_and(|track| !track.enabled)
        {
            continue;
        }
        if let Some(id) = clip.source_id.as_deref() {
            if source_indexes.contains_key(id) {
                consumers.entry(id).or_default().push(clip);
            }
        }
    }
    let mut graph = Vec::new();
    for source in &active_sources {
        let clips = consumers
            .get(source.id.as_str())
            .cloned()
            .unwrap_or_default();
        let input = source_indexes[source.id.as_str()];
        let outputs = (0..clips.len())
            .map(|index| format!("play_src_{input}_{index}"))
            .collect::<Vec<_>>();
        if outputs.len() == 1 {
            graph.push(format!("[{input}:a]anull[{}]", outputs[0]));
        } else if !outputs.is_empty() {
            graph.push(format!(
                "[{input}:a]asplit={}[{}]",
                outputs.len(),
                outputs.join("][")
            ));
        }
    }
    let any_solo_clip = project
        .clips
        .iter()
        .any(|clip| clip.kind == ClipKind::Audio && clip.solo);
    let any_solo_track = project
        .tracks
        .iter()
        .any(|track| track.kind == TrackKind::Audio && track.solo);
    let mut source_use = HashMap::<&str, usize>::new();
    let mut audio_labels = Vec::new();
    for (index, clip) in ordered.iter().enumerate() {
        if !clip.enabled || !matches!(clip.kind, ClipKind::Audio | ClipKind::Video) {
            continue;
        }
        if clip.kind == ClipKind::Video && !clip.include_audio {
            continue;
        }
        let Some(source_id) = clip.source_id.as_deref() else {
            continue;
        };
        let Some(source_index) = source_indexes.get(source_id) else {
            continue;
        };
        let branch_index = *source_use
            .entry(source_id)
            .and_modify(|count| *count += 1)
            .or_insert(0);
        let track = project.track_for_clip(clip);
        let force_mute = clip.muted
            || track.is_some_and(|track| track.muted)
            || (clip.kind == ClipKind::Audio && any_solo_clip && !clip.solo)
            || (any_solo_track && !track.is_some_and(|track| track.solo));
        let label = make_audio_filter(
            &mut graph,
            &format!("play_src_{source_index}_{branch_index}"),
            clip,
            index,
            track.map_or(1.0, |track| track.gain),
            force_mute,
        );
        audio_labels.push(label);
    }
    if audio_labels.is_empty() {
        graph.push(format!(
            "anullsrc=r={sample_rate}:cl=stereo:d={:.6}[timeline_audio]",
            project.duration
        ));
    } else {
        graph.push(format!(
            "{}amix=inputs={}:duration=longest:dropout_transition=0:normalize=0,aresample={sample_rate},aformat=sample_fmts=flt:channel_layouts=stereo,atrim=duration={:.6},asetpts=PTS-STARTPTS[timeline_audio]",
            audio_labels.iter().map(|label| format!("[{label}]")).collect::<String>(),
            audio_labels.len(),
            project.duration
        ));
    }
    args.extend([
        "-filter_complex".into(),
        graph.join(";").into(),
        "-map".into(),
        "[timeline_audio]".into(),
        "-vn".into(),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        sample_rate.to_string().into(),
        "-c:a".into(),
        "pcm_f32le".into(),
        "-f".into(),
        "f32le".into(),
        "pipe:1".into(),
    ]);
    Ok((tools.ffmpeg, args))
}

fn make_audio_filter(
    graph: &mut Vec<String>,
    input: &str,
    clip: &CompositionClip,
    index: usize,
    track_gain: f64,
    force_mute: bool,
) -> String {
    let output = format!("aclip_{index}");
    let gain = if force_mute {
        "0".to_owned()
    } else {
        let clip_gain = property_expr_at(
            clip,
            AnimatedProperty::Gain,
            &format!("(t+{:.6})", clip.timeline_start),
        );
        if (track_gain - 1.0).abs() < f64::EPSILON {
            clip_gain
        } else {
            format!("({clip_gain})*{track_gain:.6}")
        }
    };
    let mut chain = if clip.source_time_mode == SourceTimeMode::Freeze {
        let silence = format!("afreeze_{index}");
        graph.push(format!(
            "anullsrc=channel_layout=stereo:sample_rate=48000:d={:.6}[{silence}]",
            clip.duration
        ));
        format!(
            "[{silence}]atrim=duration={:.6},asetpts=PTS-STARTPTS,volume='{gain}':eval=frame",
            clip.duration
        )
    } else {
        let source_duration = clip.duration * clip.playback_rate;
        let mut chain = format!(
            "[{input}]atrim=start={:.6}:duration={source_duration:.6},asetpts=PTS-STARTPTS",
            clip.source_start
        );
        if clip.source_time_mode == SourceTimeMode::Reverse {
            chain.push_str(",areverse");
        }
        if (clip.playback_rate - 1.0).abs() >= f64::EPSILON {
            for tempo in atempo_factors(clip.playback_rate) {
                chain.push_str(&format!(",atempo={tempo:.6}"));
            }
        }
        chain.push_str(&format!(",volume='{gain}':eval=frame"));
        chain
    };
    if clip.fade_in > 0.0 {
        chain.push_str(&format!(",afade=t=in:st=0:d={:.6}", clip.fade_in));
    }
    if clip.fade_out > 0.0 {
        chain.push_str(&format!(
            ",afade=t=out:st={:.6}:d={:.6}",
            (clip.duration - clip.fade_out).max(0.0),
            clip.fade_out
        ));
    }
    chain.push_str(",aformat=channel_layouts=stereo");
    if clip.has_animation(AnimatedProperty::Pan) {
        let pan = property_expr_at(
            clip,
            AnimatedProperty::Pan,
            &format!("(t+{:.6})", clip.timeline_start),
        );
        chain.push_str(&format!(
            ",aeval=exprs='val(0)*(1-({pan}))|val(1)*(1+({pan}))':c=stereo"
        ));
    } else {
        let pan = clip.pan.clamp(-1.0, 1.0);
        let left = (1.0 - pan).min(1.0);
        let right = (1.0 + pan).min(1.0);
        chain.push_str(&format!(",pan=stereo|c0={left:.6}*c0|c1={right:.6}*c1"));
    }
    chain.push_str(&format!(
        ",adelay=delays={}:all=1[{output}]",
        (clip.timeline_start * 1000.0).round() as u64
    ));
    graph.push(chain);
    output
}

pub(crate) fn atempo_factors(mut rate: f64) -> Vec<f64> {
    let mut factors = Vec::new();
    while rate > 2.0 {
        factors.push(2.0);
        rate /= 2.0;
    }
    while rate < 0.5 {
        factors.push(0.5);
        rate /= 0.5;
    }
    if (rate - 1.0).abs() >= f64::EPSILON {
        factors.push(rate);
    }
    factors
}

fn write_video_animation_commands(
    work: &Path,
    clip: &CompositionClip,
    clip_index: usize,
    source_width: u32,
    source_height: u32,
    fps: u32,
) -> Result<(Option<PathBuf>, Option<PathBuf>)> {
    let crop_properties = [
        AnimatedProperty::CropLeft,
        AnimatedProperty::CropTop,
        AnimatedProperty::CropRight,
        AnimatedProperty::CropBottom,
    ];
    let crop_keys = crop_properties
        .iter()
        .filter_map(|property| clip.animated_properties.get(property))
        .flatten()
        .collect::<Vec<_>>();
    let blur_keys = clip
        .animated_properties
        .get(&AnimatedProperty::Blur)
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let crop_range = animated_frame_range(clip, &crop_keys, fps);
    let blur_range = animated_frame_range(clip, &blur_keys, fps);
    let crop_path = crop_range
        .as_ref()
        .map(|_| work.join(format!("crop_animation_{clip_index}.cmd")));
    let blur_path = blur_range
        .as_ref()
        .map(|_| work.join(format!("blur_animation_{clip_index}.cmd")));
    let command_count = crop_range
        .as_ref()
        .map_or(0, |(start, end)| end.saturating_sub(*start) + 1)
        .saturating_add(
            blur_range
                .as_ref()
                .map_or(0, |(start, end)| end.saturating_sub(*start) + 1),
        );
    if command_count > 500_000 {
        bail!("animated crop/blur spans too many frames for the deterministic FFmpeg command schedule");
    }
    let mut crop_text = String::new();
    if let Some((start, end)) = crop_range {
        for frame in start..=end {
            let time = clip.timeline_start + frame as f64 / f64::from(fps);
            let (width, height, x, y) = crop_geometry(
                source_width,
                source_height,
                clip.animated_value(AnimatedProperty::CropLeft, time),
                clip.animated_value(AnimatedProperty::CropTop, time),
                clip.animated_value(AnimatedProperty::CropRight, time),
                clip.animated_value(AnimatedProperty::CropBottom, time),
            )?;
            let command_start = time;
            let command_end = time + 0.5 / f64::from(fps);
            use std::fmt::Write as _;
            writeln!(
                &mut crop_text,
                "{command_start:.6}-{command_end:.6} [enter] crop@crop_{clip_index} w {width}, crop@crop_{clip_index} h {height}, crop@crop_{clip_index} x {x}, crop@crop_{clip_index} y {y};"
            )?;
        }
    }
    let mut blur_text = String::new();
    if let Some((start, end)) = blur_range {
        for frame in start..=end {
            let time = clip.timeline_start + frame as f64 / f64::from(fps);
            let sigma = clip.animated_value(AnimatedProperty::Blur, time);
            let command_start = time;
            let command_end = time + 0.5 / f64::from(fps);
            use std::fmt::Write as _;
            writeln!(
                &mut blur_text,
                "{command_start:.6}-{command_end:.6} [enter] gblur@blur_{clip_index} sigma {sigma:.6};"
            )?;
        }
    }
    if let Some(path) = &crop_path {
        fs::write(path, crop_text)
            .with_context(|| format!("writing animated crop commands {}", path.display()))?;
    }
    if let Some(path) = &blur_path {
        fs::write(path, blur_text)
            .with_context(|| format!("writing animated blur commands {}", path.display()))?;
    }
    Ok((crop_path, blur_path))
}

fn animated_frame_range(
    clip: &CompositionClip,
    keys: &[&ScalarKeyframe],
    fps: u32,
) -> Option<(usize, usize)> {
    if keys.len() < 2 || fps == 0 {
        return None;
    }
    let first = keys
        .iter()
        .map(|key| key.time)
        .fold(f64::INFINITY, f64::min);
    let last = keys
        .iter()
        .map(|key| key.time)
        .fold(f64::NEG_INFINITY, f64::max);
    let start = clip.timeline_start;
    let end = clip.timeline_start + clip.duration;
    if !first.is_finite() || !last.is_finite() || last <= start || first >= end {
        return None;
    }
    let start_frame = ((first.max(start) - start) * f64::from(fps)).ceil() as usize;
    let end_frame = ((last.min(end) - start) * f64::from(fps)).floor() as usize;
    (start_frame <= end_frame).then_some((start_frame, end_frame))
}

fn crop_geometry(
    source_width: u32,
    source_height: u32,
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
) -> Result<(u32, u32, u32, u32)> {
    if left + right >= 0.99 || top + bottom >= 0.99 {
        bail!("animated crop leaves no usable source area");
    }
    let width =
        even_dimension(f64::from(source_width) * (1.0 - left - right)).min(source_width & !1);
    let height =
        even_dimension(f64::from(source_height) * (1.0 - top - bottom)).min(source_height & !1);
    let max_x = source_width.saturating_sub(width) & !1;
    let max_y = source_height.saturating_sub(height) & !1;
    let x = ((f64::from(source_width) * left).round() as u32 & !1).min(max_x);
    let y = ((f64::from(source_height) * top).round() as u32 & !1).min(max_y);
    Ok((width.max(2), height.max(2), x, y))
}

#[cfg(test)]
fn position_expr(initial: f64, keyframes: &[PositionKeyframe]) -> String {
    let keys = keyframes
        .iter()
        .map(|key| ScalarKeyframe {
            time: key.time,
            value: key.x,
            easing_to_next: key.easing_to_next,
        })
        .collect::<Vec<_>>();
    scalar_keyframes_expr(initial, &keys)
}

fn property_expr(clip: &CompositionClip, property: AnimatedProperty) -> String {
    property_expr_at(clip, property, "t")
}

fn property_expr_at(clip: &CompositionClip, property: AnimatedProperty, time: &str) -> String {
    let keys = clip
        .animated_properties
        .get(&property)
        .cloned()
        .unwrap_or_else(|| {
            if property == AnimatedProperty::X {
                clip.position_keyframes
                    .iter()
                    .map(|key| ScalarKeyframe {
                        time: key.time,
                        value: key.x,
                        easing_to_next: key.easing_to_next,
                    })
                    .collect()
            } else {
                Vec::new()
            }
        });
    scalar_keyframes_expr_at(base_property_value(clip, property), &keys, time)
}

fn transition_opacity_expr(project: &CompositionProject, clip: &CompositionClip) -> Option<String> {
    let factors = project
        .transitions
        .iter()
        .filter(|transition| transition.kind == TransitionKind::CrossDissolve)
        .filter_map(|transition| {
            let is_incoming = transition.to_clip_id == clip.id;
            if !is_incoming {
                return None;
            }
            let start = transition.start_time;
            let end = start + transition.duration;
            let ramp = transition_progress_expr(
                transition.easing,
                format!("clip((T-{start:.6})/{:.6},0,1)", transition.duration),
            );
            Some(format!(
                "if(lt(T,{start:.6}),0,if(lt(T,{end:.6}),{ramp},1))"
            ))
        })
        .collect::<Vec<_>>();
    (!factors.is_empty()).then(|| factors.join("*"))
}

fn transition_progress_expr(easing: Easing, progress: String) -> String {
    match easing {
        Easing::Linear => progress,
        Easing::Hold => format!("if(gte({progress},1),1,0)"),
        Easing::EaseIn => format!("({progress})*({progress})"),
        Easing::EaseOut => format!("1-(1-({progress}))*(1-({progress}))"),
        Easing::EaseInOut => format!("({progress})*({progress})*(3-2*({progress}))"),
    }
}

#[cfg(test)]
fn scalar_keyframes_expr(initial: f64, keys: &[ScalarKeyframe]) -> String {
    scalar_keyframes_expr_at(initial, keys, "t")
}

fn scalar_keyframes_expr_at(initial: f64, keys: &[ScalarKeyframe], time: &str) -> String {
    if keys.is_empty() {
        return fmt(initial);
    }
    let mut expression = fmt(keys.last().unwrap().value);
    for pair in keys.windows(2).rev() {
        let from = &pair[0];
        let to = &pair[1];
        let span = to.time - from.time;
        let u = format!("(({time}-{:.6})/{:.6})", from.time, span);
        let eased = match from.easing_to_next {
            Easing::Linear => u,
            Easing::Hold => "0".to_owned(),
            Easing::EaseIn => format!("({u})*({u})"),
            Easing::EaseOut => format!("1-(1-({u}))*(1-({u}))"),
            Easing::EaseInOut => format!("({u})*({u})*(3-2*({u}))"),
        };
        expression = format!(
            "if(lt({time},{:.6}),{:.6}+({:.6})*({eased}),{expression})",
            to.time,
            from.value,
            to.value - from.value
        );
    }
    let first = &keys[0];
    format!(
        "if(lt({time},{:.6}),{:.6},{expression})",
        first.time, first.value
    )
}

fn max_animated_value(clip: &CompositionClip, property: AnimatedProperty) -> f64 {
    clip.animated_properties
        .get(&property)
        .into_iter()
        .flatten()
        .map(|key| key.value)
        .fold(base_property_value(clip, property), f64::max)
}

pub(crate) fn animated_property_range(property: AnimatedProperty) -> (f64, f64) {
    match property {
        AnimatedProperty::X | AnimatedProperty::Y => (-32_768.0, 32_768.0),
        AnimatedProperty::ScaleX | AnimatedProperty::ScaleY => (0.01, 16.0),
        AnimatedProperty::Rotation => (-36_000.0, 36_000.0),
        AnimatedProperty::Opacity => (0.0, 1.0),
        AnimatedProperty::CropLeft
        | AnimatedProperty::CropTop
        | AnimatedProperty::CropRight
        | AnimatedProperty::CropBottom => (0.0, 0.95),
        AnimatedProperty::Blur => (0.0, 100.0),
        AnimatedProperty::MaskX | AnimatedProperty::MaskY => (0.0, 1.0),
        AnimatedProperty::MaskWidth | AnimatedProperty::MaskHeight => (0.01, 1.0),
        AnimatedProperty::MaskFeather => (0.0, 0.5),
        AnimatedProperty::Gain => (0.0, 16.0),
        AnimatedProperty::Pan => (-1.0, 1.0),
        AnimatedProperty::FontSize => (1.0, 512.0),
        AnimatedProperty::OutlineWidth => (0.0, 32.0),
        AnimatedProperty::ShadowX | AnimatedProperty::ShadowY => (-128.0, 128.0),
    }
}

fn base_property_value(clip: &CompositionClip, property: AnimatedProperty) -> f64 {
    match property {
        AnimatedProperty::X => clip.x,
        AnimatedProperty::Y => clip.y,
        AnimatedProperty::ScaleX => clip.scale_x,
        AnimatedProperty::ScaleY => clip.scale_y,
        AnimatedProperty::Rotation => clip.rotation_degrees,
        AnimatedProperty::Opacity => clip.opacity,
        AnimatedProperty::CropLeft => clip.crop_left,
        AnimatedProperty::CropTop => clip.crop_top,
        AnimatedProperty::CropRight => clip.crop_right,
        AnimatedProperty::CropBottom => clip.crop_bottom,
        AnimatedProperty::Blur => clip.blur,
        AnimatedProperty::MaskX => clip.mask.as_ref().map_or(0.1, |mask| mask.x),
        AnimatedProperty::MaskY => clip.mask.as_ref().map_or(0.1, |mask| mask.y),
        AnimatedProperty::MaskWidth => clip.mask.as_ref().map_or(0.8, |mask| mask.width),
        AnimatedProperty::MaskHeight => clip.mask.as_ref().map_or(0.8, |mask| mask.height),
        AnimatedProperty::MaskFeather => clip.mask.as_ref().map_or(0.0, |mask| mask.feather),
        AnimatedProperty::Gain => clip.gain,
        AnimatedProperty::Pan => clip.pan,
        AnimatedProperty::FontSize => f64::from(clip.font_size),
        AnimatedProperty::OutlineWidth => clip.outline_width,
        AnimatedProperty::ShadowX => clip.shadow_x,
        AnimatedProperty::ShadowY => clip.shadow_y,
    }
}

fn ensure_mask_for_property(clip: &mut CompositionClip, property: AnimatedProperty) {
    if matches!(
        property,
        AnimatedProperty::MaskX
            | AnimatedProperty::MaskY
            | AnimatedProperty::MaskWidth
            | AnimatedProperty::MaskHeight
            | AnimatedProperty::MaskFeather
    ) {
        clip.mask.get_or_insert_with(VisualMask::default);
    }
}

fn set_base_property_value(clip: &mut CompositionClip, property: AnimatedProperty, value: f64) {
    match property {
        AnimatedProperty::X => clip.x = value,
        AnimatedProperty::Y => clip.y = value,
        AnimatedProperty::ScaleX => clip.scale_x = value,
        AnimatedProperty::ScaleY => clip.scale_y = value,
        AnimatedProperty::Rotation => clip.rotation_degrees = value,
        AnimatedProperty::Opacity => clip.opacity = value,
        AnimatedProperty::CropLeft => clip.crop_left = value,
        AnimatedProperty::CropTop => clip.crop_top = value,
        AnimatedProperty::CropRight => clip.crop_right = value,
        AnimatedProperty::CropBottom => clip.crop_bottom = value,
        AnimatedProperty::Blur => clip.blur = value,
        AnimatedProperty::MaskX => {
            clip.mask.get_or_insert_with(VisualMask::default).x = value;
        }
        AnimatedProperty::MaskY => {
            clip.mask.get_or_insert_with(VisualMask::default).y = value;
        }
        AnimatedProperty::MaskWidth => {
            clip.mask.get_or_insert_with(VisualMask::default).width = value;
        }
        AnimatedProperty::MaskHeight => {
            clip.mask.get_or_insert_with(VisualMask::default).height = value;
        }
        AnimatedProperty::MaskFeather => {
            clip.mask.get_or_insert_with(VisualMask::default).feather = value;
        }
        AnimatedProperty::Gain => clip.gain = value,
        AnimatedProperty::Pan => clip.pan = value,
        AnimatedProperty::FontSize => clip.font_size = value.round().clamp(1.0, 512.0) as u32,
        AnimatedProperty::OutlineWidth => clip.outline_width = value,
        AnimatedProperty::ShadowX => clip.shadow_x = value,
        AnimatedProperty::ShadowY => clip.shadow_y = value,
    }
}

fn evaluate_legacy_x(keys: &[PositionKeyframe], time: f64, base: f64) -> f64 {
    let converted = keys
        .iter()
        .map(|key| ScalarKeyframe {
            time: key.time,
            value: key.x,
            easing_to_next: key.easing_to_next,
        })
        .collect::<Vec<_>>();
    evaluate_keyframes(&converted, time, base)
}

fn evaluate_keyframes(keys: &[ScalarKeyframe], time: f64, base: f64) -> f64 {
    if keys.is_empty() {
        return base;
    }
    if time <= keys[0].time {
        return keys[0].value;
    }
    let next = keys.partition_point(|key| key.time <= time);
    if next >= keys.len() {
        return keys.last().map_or(base, |key| key.value);
    }
    let from = &keys[next - 1];
    let to = &keys[next];
    if from.easing_to_next == Easing::Hold {
        return from.value;
    }
    let u = ((time - from.time) / (to.time - from.time)).clamp(0.0, 1.0);
    let t = match from.easing_to_next {
        Easing::Linear => u,
        Easing::Hold => 0.0,
        Easing::EaseIn => u * u,
        Easing::EaseOut => 1.0 - (1.0 - u) * (1.0 - u),
        Easing::EaseInOut => u * u * (3.0 - 2.0 * u),
    };
    from.value + (to.value - from.value) * t
}

fn default_font() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(windows) = std::env::var_os("WINDIR") {
        candidates.push(PathBuf::from(windows).join("Fonts/arial.ttf"));
    }
    candidates.extend([
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
    ]);
    candidates.into_iter().find(|path| path.is_file())
}

fn escape_filter_path(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace(':', "\\:")
        .replace('\'', "\\'")
        .replace(',', "\\,")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

fn even_dimension(value: f64) -> u32 {
    let rounded = value.round().max(2.0) as u32;
    if rounded % 2 == 0 {
        rounded
    } else {
        rounded + 1
    }
}

fn fmt(value: f64) -> String {
    format!("{value:.6}")
}

fn finite_range(value: f64, min: f64, max: f64, label: &str) -> Result<()> {
    if !value.is_finite() || value < min || value > max {
        bail!("{label} must be finite and in {min}..={max}");
    }
    Ok(())
}

fn validate_color(value: &str) -> Result<()> {
    if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("colors must be six hexadecimal RGB digits");
    }
    Ok(())
}

pub(crate) fn effect_parameter_range(
    kind: &VisualEffectKind,
    parameter: EffectParameter,
) -> Option<(f64, f64)> {
    match (kind, parameter) {
        (VisualEffectKind::Brightness { .. }, EffectParameter::Amount) => Some((-1.0, 1.0)),
        (
            VisualEffectKind::Contrast { .. } | VisualEffectKind::Saturation { .. },
            EffectParameter::Amount,
        ) => Some((0.0, 4.0)),
        (VisualEffectKind::Gamma { .. }, EffectParameter::Amount) => Some((0.1, 10.0)),
        (VisualEffectKind::Hue { .. }, EffectParameter::HueDegrees) => Some((-180.0, 180.0)),
        _ => None,
    }
}

fn effect_parameter_value(kind: &VisualEffectKind, parameter: EffectParameter) -> Option<f64> {
    match (kind, parameter) {
        (VisualEffectKind::Brightness { amount }, EffectParameter::Amount)
        | (VisualEffectKind::Contrast { amount }, EffectParameter::Amount)
        | (VisualEffectKind::Saturation { amount }, EffectParameter::Amount)
        | (VisualEffectKind::Gamma { amount }, EffectParameter::Amount) => Some(*amount),
        (VisualEffectKind::Hue { degrees }, EffectParameter::HueDegrees) => Some(*degrees),
        _ => None,
    }
}

fn effect_parameter_expr(effect: &VisualEffectInstance, parameter: EffectParameter) -> String {
    let Some(initial) = effect_parameter_value(&effect.kind, parameter) else {
        return "0".into();
    };
    let keys = effect
        .animated_parameters
        .get(&parameter)
        .map(Vec::as_slice)
        .unwrap_or_default();
    scalar_keyframes_expr_at(initial, keys, "t")
}

fn validate_effect_parameter_keyframes(
    effect: &VisualEffectInstance,
    composition_duration: f64,
) -> Result<()> {
    if effect.animated_parameters.len() > 8 {
        bail!("a visual effect has too many animated parameters");
    }
    for (parameter, keys) in &effect.animated_parameters {
        if effect_parameter_range(&effect.kind, *parameter).is_none() {
            bail!(
                "{} cannot be animated on this visual effect",
                parameter.label()
            );
        }
        if keys.len() > 16_384 {
            bail!("a visual effect has too many keyframes");
        }
        let (minimum, maximum) = effect_parameter_range(&effect.kind, *parameter).unwrap();
        let mut previous_time = -1.0;
        for key in keys {
            finite_range(key.time, 0.0, composition_duration, "effect keyframe time")?;
            finite_range(key.value, minimum, maximum, "effect keyframe value")?;
            if key.time <= previous_time {
                bail!("effect keyframes must have strictly increasing times");
            }
            previous_time = key.time;
        }
    }
    Ok(())
}

fn visual_effect_filter(effect: &VisualEffectInstance) -> String {
    match &effect.kind {
        VisualEffectKind::GaussianBlur { sigma } if *sigma > 0.0 => {
            format!("gblur=sigma={sigma:.6}")
        }
        VisualEffectKind::GaussianBlur { .. } => "null".into(),
        VisualEffectKind::Brightness { amount: _ } => {
            let expr = effect_parameter_expr(effect, EffectParameter::Amount);
            format!("eq=brightness='{expr}':eval=frame")
        }
        VisualEffectKind::Contrast { amount: _ } => {
            let expr = effect_parameter_expr(effect, EffectParameter::Amount);
            format!("eq=contrast='{expr}':eval=frame")
        }
        VisualEffectKind::Saturation { amount: _ } => {
            let expr = effect_parameter_expr(effect, EffectParameter::Amount);
            format!("eq=saturation='{expr}':eval=frame")
        }
        VisualEffectKind::Hue { degrees: _ } => {
            let expr = effect_parameter_expr(effect, EffectParameter::HueDegrees);
            // FFmpeg's hue filter evaluates its expression per frame and has
            // no `eval` option (unlike `eq`).
            format!("hue=h='{expr}'")
        }
        VisualEffectKind::Sharpen { amount } => {
            format!("unsharp=5:5:{amount:.6}:5:5:0")
        }
        VisualEffectKind::Gamma { amount: _ } => {
            let expr = effect_parameter_expr(effect, EffectParameter::Amount);
            format!("eq=gamma='{expr}':eval=frame")
        }
        VisualEffectKind::ChromaKey {
            key_color,
            tolerance,
            softness,
        } => format!(
            "format=yuva444p,chromakey=0x{key_color}:similarity={tolerance:.6}:blend={softness:.6},format=rgba"
        ),
    }
}

fn validate_visual_effect(effect: &VisualEffectKind) -> Result<()> {
    match effect {
        VisualEffectKind::GaussianBlur { sigma } => {
            finite_range(*sigma, 0.0, 100.0, "effect Gaussian blur sigma")?;
        }
        VisualEffectKind::Brightness { amount } => {
            finite_range(*amount, -1.0, 1.0, "effect brightness")?;
        }
        VisualEffectKind::Contrast { amount } => {
            finite_range(*amount, 0.0, 4.0, "effect contrast")?;
        }
        VisualEffectKind::Saturation { amount } => {
            finite_range(*amount, 0.0, 4.0, "effect saturation")?;
        }
        VisualEffectKind::Hue { degrees } => {
            finite_range(*degrees, -180.0, 180.0, "effect hue")?;
        }
        VisualEffectKind::Sharpen { amount } => {
            finite_range(*amount, 0.0, 5.0, "effect sharpen")?;
        }
        VisualEffectKind::Gamma { amount } => {
            finite_range(*amount, 0.1, 10.0, "effect gamma")?;
        }
        VisualEffectKind::ChromaKey {
            key_color,
            tolerance,
            softness,
        } => {
            validate_color(key_color)?;
            finite_range(*tolerance, 0.0, 1.0, "chroma-key tolerance")?;
            finite_range(*softness, 0.0, 1.0, "chroma-key softness")?;
        }
    }
    Ok(())
}

fn unique_id(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    )
}

fn validate_nested_graph(
    project: &CompositionProject,
    ancestors: &mut Vec<String>,
    completed: &mut HashSet<String>,
) -> Result<()> {
    if ancestors.iter().any(|id| id == &project.composition_id) {
        bail!("nested composition cycle detected at {}", project.name);
    }
    if ancestors.len() >= 16 {
        bail!("nested compositions may not exceed 16 levels");
    }
    if completed.contains(&project.composition_id) {
        return Ok(());
    }
    ancestors.push(project.composition_id.clone());
    for source in &project.sources {
        let Some(path) = source.nested_composition_path.as_deref() else {
            continue;
        };
        // Offline child projects remain inspectable. The parent render reports
        // the precise missing reference if it is used.
        if !path.is_file() {
            continue;
        }
        let child = CompositionProject::load_with_nested_validation(path, false)
            .with_context(|| format!("loading nested composition {}", path.display()))?;
        validate_nested_graph(&child, ancestors, completed)?;
    }
    ancestors.pop();
    completed.insert(project.composition_id.clone());
    Ok(())
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let base = base.canonicalize().unwrap_or_else(|_| base.to_path_buf());
    let target = target
        .canonicalize()
        .unwrap_or_else(|_| target.to_path_buf());
    if !base.is_absolute() || !target.is_absolute() {
        return None;
    }
    let a: Vec<_> = base.components().collect();
    let b: Vec<_> = target.components().collect();
    let mut common = 0;
    while common < a.len() && common < b.len() && component_equal(a[common], b[common]) {
        common += 1;
    }
    if common == 0 {
        return None;
    }
    let mut result = PathBuf::new();
    for component in &a[common..] {
        if matches!(component, Component::Normal(_)) {
            result.push("..");
        }
    }
    for component in &b[common..] {
        result.push(component.as_os_str());
    }
    Some(result)
}

fn component_equal(a: Component<'_>, b: Component<'_>) -> bool {
    #[cfg(windows)]
    {
        a.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn project() -> CompositionProject {
        let mut project = CompositionProject::new();
        project.width = 1080;
        project.height = 1920;
        project.fps = 60;
        project.duration = 12.0;
        project
    }

    fn source() -> MediaSource {
        MediaSource {
            id: "source-1".into(),
            path: "input.mp4".into(),
            display_name: "input.mp4".into(),
            has_video: true,
            has_audio: true,
            duration: 12.0,
            width: Some(1920),
            height: Some(1080),
            ..Default::default()
        }
    }

    #[test]
    fn standalone_composition_validates_vertical_and_square_dimensions_without_watch_inputs() {
        let mut value = project();
        value.validate().unwrap();
        value.width = 1080;
        value.height = 1080;
        value.validate().unwrap();
        value.width = 1920;
        value.height = 1080;
        value.validate().unwrap();
        value.width = 1921;
        assert!(value.validate().is_err());
    }

    #[test]
    fn recovery_checkpoint_is_versioned_atomic_and_does_not_replace_project_file() {
        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("draft.sonocomp");
        let checkpoint = directory.path().join("recovery").join("draft.json");
        let original = project();
        original.save(&project_path).unwrap();
        let canonical_project = fs::read(&project_path).unwrap();

        write_recovery_snapshot(&checkpoint, &original, None).unwrap();
        let recovered = load_recovery_snapshot(&checkpoint).unwrap();
        assert_eq!(recovered.project, original);
        assert!(recovered.checkpoint_unix_ms > 0);
        assert_eq!(fs::read(&project_path).unwrap(), canonical_project);

        let mut edited = original.clone();
        edited.duration = 19.0;
        write_recovery_snapshot(&checkpoint, &edited, None).unwrap();
        assert_eq!(load_recovery_snapshot(&checkpoint).unwrap().project, edited);
        assert_eq!(fs::read(&project_path).unwrap(), canonical_project);

        let bytes = fs::read(&checkpoint).unwrap();
        let mut invalid = edited.clone();
        invalid.width = 1921;
        assert!(invalid.save(&project_path).is_err());
        assert_eq!(fs::read(&project_path).unwrap(), canonical_project);

        assert!(write_recovery_snapshot(&checkpoint, &invalid, None).is_err());
        assert_eq!(fs::read(&checkpoint).unwrap(), bytes);
        assert_eq!(load_recovery_snapshot(&checkpoint).unwrap().project, edited);
        assert_eq!(fs::read(&project_path).unwrap(), canonical_project);

        let mut invalid: Value = serde_json::from_slice(&bytes).unwrap();
        invalid["schema_version"] = 999.into();
        fs::write(&checkpoint, serde_json::to_vec(&invalid).unwrap()).unwrap();
        assert!(load_recovery_snapshot(&checkpoint).is_err());
        assert!(fs::read(&checkpoint).is_ok());
        assert_eq!(fs::read(&project_path).unwrap(), canonical_project);
    }

    #[test]
    fn split_preserves_source_continuity_and_duplicate_visuals_are_muted() {
        let mut value = project();
        value.sources.push(source());
        let id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value.clips[0].duration = 10.0;
        let right = value.split_clip(&id, 4.0).unwrap();
        let right_clip = value.clips.iter().find(|clip| clip.id == right).unwrap();
        assert_eq!(right_clip.source_start, 4.0);
        assert_eq!(right_clip.timeline_start, 4.0);
        let duplicate = value.duplicate_clip(&id).unwrap();
        assert!(
            !value
                .clips
                .iter()
                .find(|clip| clip.id == duplicate)
                .unwrap()
                .include_audio
        );
    }

    #[test]
    fn track_order_moves_only_within_its_visual_or_audio_stack() {
        let mut value = project();
        let visual_a = value.first_track_id(TrackKind::Visual).unwrap();
        let visual_b = value.add_track(TrackKind::Visual, "Foreground").unwrap();
        let audio_a = value.first_track_id(TrackKind::Audio).unwrap();
        value.move_track(&visual_b, -1).unwrap();
        assert!(
            value
                .tracks
                .iter()
                .find(|track| track.id == visual_b)
                .unwrap()
                .order
                < value
                    .tracks
                    .iter()
                    .find(|track| track.id == visual_a)
                    .unwrap()
                    .order
        );
        assert_eq!(
            value
                .tracks
                .iter()
                .find(|track| track.id == audio_a)
                .unwrap()
                .order,
            0
        );
        value.validate().unwrap();
    }

    #[test]
    fn linked_av_moves_splits_unlinks_and_deletes_as_one_timeline_object() {
        let mut value = project();
        let media = source();
        let source_id = media.id.clone();
        value.sources.push(media);
        let (video_id, audio_id) = value.add_linked_av_clip(&source_id, 1.0).unwrap();
        value.move_clip_to(&video_id, 0.5).unwrap();
        let video = value.clips.iter().find(|clip| clip.id == video_id).unwrap();
        let audio = value.clips.iter().find(|clip| clip.id == audio_id).unwrap();
        assert_eq!(video.timeline_start, audio.timeline_start);
        assert_eq!(video.link_group_id, audio.link_group_id);

        value.slip_clip_by(&video_id, 0.5).unwrap();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == video_id)
                .unwrap()
                .source_start,
            0.5
        );
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == audio_id)
                .unwrap()
                .source_start,
            0.5
        );
        let right_video = value.split_clip(&video_id, 5.0).unwrap();
        let right_audio = value
            .clips
            .iter()
            .find(|clip| clip.id != audio_id && clip.kind == ClipKind::Audio)
            .unwrap()
            .id
            .clone();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == right_video)
                .unwrap()
                .link_group_id,
            value
                .clips
                .iter()
                .find(|clip| clip.id == right_audio)
                .unwrap()
                .link_group_id
        );
        value.unlink_clip(&right_video).unwrap();
        assert!(value
            .clips
            .iter()
            .find(|clip| clip.id == right_video)
            .unwrap()
            .link_group_id
            .is_none());
        value.remove_clip(&video_id).unwrap();
        assert!(!value.clips.iter().any(|clip| clip.id == audio_id));
        value.validate().unwrap();
    }

    #[test]
    fn user_groups_move_and_ungroup_as_one_timeline_object() {
        let mut value = project();
        value.sources.push(source());
        let first = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let second = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        for clip in &mut value.clips {
            clip.duration = 4.0;
        }

        let group = value.group_clips(&[first.clone(), second.clone()]).unwrap();
        assert_ne!(group, "");
        assert!(value
            .clips
            .iter()
            .all(|clip| clip.group_id.as_deref() == Some(group.as_str())));

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("grouped.sonocomp");
        value.save(&path).unwrap();
        value = CompositionProject::load(&path).unwrap();
        assert!(value
            .clips
            .iter()
            .all(|clip| clip.group_id.as_deref() == Some(group.as_str())));

        value.move_clip_to(&first, 2.0).unwrap();
        assert!(value.clips.iter().all(|clip| clip.timeline_start == 2.0));

        value.ungroup_clip(&first).unwrap();
        value.move_clip_to(&first, 3.0).unwrap();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == first)
                .unwrap()
                .timeline_start,
            3.0
        );
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == second)
                .unwrap()
                .timeline_start,
            2.0
        );
        value.validate().unwrap();
    }

    #[test]
    fn grouping_includes_linked_av_partners_and_group_deletion_is_atomic() {
        let mut value = project();
        value.sources.push(source());
        let (video, audio) = value.add_linked_av_clip("source-1", 0.0).unwrap();
        let independent = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        for clip in &mut value.clips {
            clip.duration = 4.0;
        }

        let group = value
            .group_clips(&[video.clone(), independent.clone()])
            .unwrap();
        assert_eq!(value.clips.len(), 3);
        assert!(value
            .clips
            .iter()
            .all(|clip| clip.group_id.as_deref() == Some(group.as_str())));
        assert_ne!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == video)
                .unwrap()
                .group_id,
            value
                .clips
                .iter()
                .find(|clip| clip.id == video)
                .unwrap()
                .link_group_id
        );

        value.move_clip_to(&video, 1.0).unwrap();
        assert!(value.clips.iter().all(|clip| clip.timeline_start == 1.0));
        value.remove_clip(&independent).unwrap();
        assert!(value.clips.is_empty());
        assert_ne!(video, audio);
    }

    #[test]
    fn group_operations_reject_locked_tracks_without_partial_mutation() {
        let mut value = project();
        value.sources.push(source());
        let first = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let second = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        for clip in &mut value.clips {
            clip.duration = 4.0;
        }
        let track_id = value.first_track_id(TrackKind::Visual).unwrap();
        value
            .tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .unwrap()
            .locked = true;

        assert!(value.group_clips(&[first.clone(), second.clone()]).is_err());
        assert!(value.clips.iter().all(|clip| clip.group_id.is_none()));
        assert!(value.move_clip_to(&first, 1.0).is_err());
        assert!(value.clips.iter().all(|clip| clip.timeline_start == 0.0));
    }

    #[test]
    fn copies_of_group_members_are_independent_clips() {
        let mut value = project();
        value.sources.push(source());
        let first = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let second = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        for clip in &mut value.clips {
            clip.duration = 4.0;
        }
        value.group_clips(&[first.clone(), second]).unwrap();

        let duplicate = value.duplicate_clip(&first).unwrap();
        assert!(value
            .clips
            .iter()
            .find(|clip| clip.id == duplicate)
            .unwrap()
            .group_id
            .is_none());
        value.validate().unwrap();
    }

    #[test]
    fn source_range_insertion_preserves_distinct_source_and_timeline_times() {
        let mut value = project();
        value.sources.push(source());
        let clip_id = value
            .add_clip_from_range("source-1", ClipKind::Video, 3.0, 2.0, 5.0, None)
            .unwrap();
        let clip = value.clips.iter().find(|clip| clip.id == clip_id).unwrap();
        assert_eq!(clip.timeline_start, 3.0);
        assert_eq!(clip.source_start, 2.0);
        assert_eq!(clip.duration, 3.0);
        assert!(value
            .add_clip_from_range("source-1", ClipKind::Video, 10.0, 2.0, 5.0, None)
            .is_err());
        assert!(value
            .add_clip_from_range("source-1", ClipKind::Video, 0.0, 10.0, 13.0, None)
            .is_err());
        assert_eq!(
            value.clips.len(),
            1,
            "rejected source ranges are transactional"
        );
    }

    #[test]
    fn ordered_visual_effects_persist_reorder_bypass_and_feed_the_compositor() {
        let mut value = project();
        let source_id = value.add_solid_color_source("4080C0").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let brightness_id = value
            .add_visual_effect(&clip_id, VisualEffectKind::Brightness { amount: 0.2 })
            .unwrap();
        let saturation_id = value
            .add_visual_effect(&clip_id, VisualEffectKind::Saturation { amount: 1.4 })
            .unwrap();
        let gamma_id = value
            .add_visual_effect(&clip_id, VisualEffectKind::Gamma { amount: 1.25 })
            .unwrap();
        value
            .move_visual_effect(&clip_id, &saturation_id, -1)
            .unwrap();
        assert_eq!(
            value.clips[0]
                .effects
                .iter()
                .map(|effect| effect.id.as_str())
                .collect::<Vec<_>>(),
            vec![
                saturation_id.as_str(),
                brightness_id.as_str(),
                gamma_id.as_str()
            ]
        );
        value.clips[0].effects[0].enabled = false;
        value.validate().unwrap();

        let temp = tempfile::tempdir().unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(!graph.contains("saturation=1.400000"));
        assert!(graph.contains("eq=brightness='0.200000':eval=frame"));
        assert!(graph.contains("eq=gamma='1.250000':eval=frame"));

        value.clips[0].effects[0].enabled = true;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(
            graph.find("eq=saturation='1.400000':eval=frame").unwrap()
                < graph.find("eq=brightness='0.200000':eval=frame").unwrap()
        );
        let duplicate_id = value.duplicate_clip(&clip_id).unwrap();
        let right_id = value.split_clip(&duplicate_id, 6.0).unwrap();
        let all_effect_ids = value
            .clips
            .iter()
            .flat_map(|clip| clip.effects.iter().map(|effect| effect.id.as_str()))
            .collect::<HashSet<_>>();
        assert_eq!(all_effect_ids.len(), 9);
        value.validate().unwrap();
        let project_path = temp.path().join("effects.sonocomp");
        value.save(&project_path).unwrap();
        let reopened = CompositionProject::load(&project_path).unwrap();
        assert_eq!(reopened, value);
        assert!(reopened.clips.iter().any(|clip| clip.id == right_id));
    }

    #[test]
    fn effect_parameter_keyframes_validate_persist_and_feed_frame_evaluated_filters() {
        let mut value = project();
        let source_id = value.add_solid_color_source("404040").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let effect_id = value
            .add_visual_effect(&clip_id, VisualEffectKind::Brightness { amount: 0.0 })
            .unwrap();
        value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                0.0,
                -0.2,
                Easing::EaseInOut,
            )
            .unwrap();
        value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                2.0,
                0.4,
                Easing::Linear,
            )
            .unwrap();
        // Setting a key at the same timestamp edits it instead of creating an
        // ambiguous duplicate-time interpolation point.
        value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                2.0,
                0.3,
                Easing::Linear,
            )
            .unwrap();
        assert_eq!(
            value.clips[0].effects[0].animated_parameters[&EffectParameter::Amount].len(),
            2
        );
        assert_eq!(
            value.clips[0].effects[0].animated_parameters[&EffectParameter::Amount][1].value,
            0.3
        );
        value
            .edit_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                2.0,
                1.5,
                0.35,
                Easing::EaseOut,
            )
            .unwrap();
        assert_eq!(
            value.clips[0].effects[0].animated_parameters[&EffectParameter::Amount][1].time,
            1.5
        );
        assert_eq!(
            value.clips[0].effects[0].animated_parameters[&EffectParameter::Amount][1]
                .easing_to_next,
            Easing::EaseOut
        );
        let before_conflicting_edit = value.clone();
        assert!(value
            .edit_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                1.5,
                0.0,
                0.1,
                Easing::Linear,
            )
            .is_err());
        assert_eq!(value, before_conflicting_edit);
        let effect = &value.clips[0].effects[0];
        let filter = visual_effect_filter(effect);
        assert!(filter.contains("if(lt(t,1.500000)"));
        assert!(filter.ends_with(":eval=frame"));

        let before_invalid_edit = value.clone();
        assert!(value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::HueDegrees,
                1.0,
                20.0,
                Easing::Linear,
            )
            .is_err());
        assert_eq!(value, before_invalid_edit);

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("animated-effect.sonocomp");
        value.save(&path).unwrap();
        assert_eq!(CompositionProject::load(&path).unwrap(), value);

        value.tracks[0].locked = true;
        let before_locked_edit = value.clone();
        assert!(value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                1.0,
                0.1,
                Easing::Linear,
            )
            .is_err());
        assert!(value
            .edit_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                1.5,
                1.25,
                0.2,
                Easing::Linear,
            )
            .is_err());
        assert_eq!(value, before_locked_edit);
    }

    #[test]
    fn adjustment_layers_are_source_free_editable_visual_clips() {
        let mut value = project();
        let adjustment_id = value.add_adjustment_clip(1.0, 3.0).unwrap();
        let effect_id = value
            .add_visual_effect(
                &adjustment_id,
                VisualEffectKind::Brightness { amount: 0.25 },
            )
            .unwrap();
        let adjustment = value
            .clips
            .iter()
            .find(|clip| clip.id == adjustment_id)
            .unwrap();
        assert_eq!(adjustment.kind, ClipKind::Adjustment);
        assert!(adjustment.source_id.is_none());
        assert_eq!(adjustment.track_id, value.first_track_id(TrackKind::Visual));
        assert_eq!(adjustment.effects[0].id, effect_id);

        let right_id = value.split_clip(&adjustment_id, 2.0).unwrap();
        let left = value
            .clips
            .iter()
            .find(|clip| clip.id == adjustment_id)
            .unwrap();
        let right = value.clips.iter().find(|clip| clip.id == right_id).unwrap();
        assert_eq!(left.source_start, 0.0);
        assert_eq!(right.source_start, 0.0);
        assert_eq!(left.kind, ClipKind::Adjustment);
        assert_eq!(right.kind, ClipKind::Adjustment);

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("adjustment.sonocomp");
        value.save(&path).unwrap();
        let reopened = CompositionProject::load(&path).unwrap();
        assert_eq!(reopened, value);
        assert_eq!(COMPOSITION_SCHEMA_VERSION, 21);
    }

    #[test]
    fn visual_effect_validation_rejects_invalid_values_and_duplicate_ids() {
        let mut value = project();
        let source_id = value.add_solid_color_source("4080C0").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        value.clips[0].effects.push(VisualEffectInstance {
            id: "bad-effect".into(),
            enabled: true,
            animated_parameters: BTreeMap::new(),
            kind: VisualEffectKind::ChromaKey {
                key_color: "green".into(),
                tolerance: 0.2,
                softness: 0.1,
            },
        });
        assert!(value.validate().is_err());
        value.clips[0].effects.clear();
        let effect = VisualEffectInstance {
            id: "duplicate".into(),
            enabled: true,
            animated_parameters: BTreeMap::new(),
            kind: VisualEffectKind::Contrast { amount: 1.0 },
        };
        value.clips[0].effects = vec![effect.clone(), effect];
        assert!(value.validate().is_err());
        value.clips[0].effects.clear();
        value.clips[0].effects.push(VisualEffectInstance {
            id: "invalid-gamma".into(),
            enabled: true,
            animated_parameters: BTreeMap::new(),
            kind: VisualEffectKind::Gamma { amount: 0.0 },
        });
        assert!(value.validate().is_err());
        value.clips[0].effects.clear();
        value
            .add_visual_effect(
                &clip_id,
                VisualEffectKind::ChromaKey {
                    key_color: "00FF00".into(),
                    tolerance: 0.2,
                    softness: 0.1,
                },
            )
            .unwrap();
        value.validate().unwrap();
    }

    #[test]
    fn visual_masks_validate_and_share_the_generalized_keyframe_graph() {
        let mut value = project();
        let source_id = value.add_solid_color_source("30B060").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let clip = value
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .unwrap();
        clip.mask = Some(VisualMask {
            shape: VisualMaskShape::Ellipse,
            x: 0.2,
            y: 0.1,
            width: 0.6,
            height: 0.8,
            feather: 0.04,
            inverted: true,
        });
        clip.add_keyframe(AnimatedProperty::MaskX, 1.0, 0.25, Easing::EaseInOut)
            .unwrap();
        clip.add_keyframe(AnimatedProperty::MaskFeather, 1.0, 0.08, Easing::Linear)
            .unwrap();
        value.validate().unwrap();

        let temp = tempfile::tempdir().unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(graph.contains("geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*("));
        assert!(
            graph.contains("sqrt(pow("),
            "ellipse equation missing from compositor"
        );
        assert!(
            graph.contains("T"),
            "animated mask does not use the frame clock"
        );
        assert!(
            graph.contains("min(W,H)"),
            "feather is not resolution-relative"
        );

        let mut invalid = value.clone();
        invalid.clips[0].mask.as_mut().unwrap().x = 0.8;
        assert!(invalid.validate().is_err());
    }

    #[test]
    fn transitions_are_first_class_persistent_and_compile_to_shared_compositor() {
        let mut value = project();
        value.width = 640;
        value.height = 360;
        value.fps = 30;
        value.duration = 4.0;
        let red_source = value.add_solid_color_source("E02020").unwrap();
        let blue_source = value.add_solid_color_source("2040E0").unwrap();
        let from_id = value.add_clip(&red_source, ClipKind::Video, 0.0).unwrap();
        let to_id = value.add_clip(&blue_source, ClipKind::Video, 1.0).unwrap();
        value
            .clips
            .iter_mut()
            .find(|clip| clip.id == from_id)
            .unwrap()
            .duration = 2.0;
        value.validate().unwrap();
        let transition_id = value
            .add_transition_between(&from_id, &to_id, TransitionKind::CrossDissolve, 1.0)
            .unwrap();
        value
            .set_transition_easing(&transition_id, Easing::EaseInOut)
            .unwrap();
        assert_eq!(value.transitions[0].start_time, 1.0);
        value.validate().unwrap();

        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("transition.sonocomp");
        value.save(&path).unwrap();
        assert_eq!(CompositionProject::load(&path).unwrap(), value);

        value
            .update_transition(&transition_id, TransitionKind::DipToBlack, 0.8)
            .unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), Some(1.5)).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(graph.contains("dip_overlay_0"));
        assert!(graph.contains("color=c=black"));
        assert!(graph.contains("3-2*("));

        value
            .update_transition(&transition_id, TransitionKind::DipToWhite, 0.8)
            .unwrap();
        value.remove_transition(&transition_id).unwrap();
        assert!(value.transitions.is_empty());
        value.validate().unwrap();

        let mut no_overlap = value.clone();
        no_overlap
            .clips
            .iter_mut()
            .find(|clip| clip.id == to_id)
            .unwrap()
            .timeline_start = 3.0;
        assert!(no_overlap
            .add_transition_between(&from_id, &to_id, TransitionKind::CrossDissolve, 0.3,)
            .is_err());
    }

    #[test]
    fn source_time_mapping_uses_timeline_duration_and_explicit_audio_tempo() {
        let mut value = project();
        value.sources.push(source());
        let clip_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value.clips[0].duration = 4.0;
        value.clips[0].include_audio = true;
        value
            .set_clip_source_time(&clip_id, SourceTimeMode::Forward, 2.0)
            .unwrap();
        value.validate().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let source_path = temp.path().join("composition-source.mp4");
        fs::write(&source_path, b"validation graph only").unwrap();
        value.sources[0].path = source_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(graph.contains("trim=start=0.000000:duration=8.000000"));
        assert!(graph.contains("setpts=(PTS-STARTPTS)/2.000000"));
        assert!(graph.contains("atempo=2.000000"));

        value
            .set_clip_source_time(&clip_id, SourceTimeMode::Reverse, 0.5)
            .unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(graph.contains(",reverse,"));
        assert!(graph.contains("atempo=0.500000"));

        value.clips[0].source_start = 2.0;
        value
            .set_clip_source_time(&clip_id, SourceTimeMode::Freeze, 1.0)
            .unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let graph = args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .unwrap()[1]
            .to_string_lossy();
        assert!(graph.contains("tpad=stop_mode=clone"));
        assert!(graph.contains("anullsrc=channel_layout=stereo"));
        assert!((atempo_factors(0.1).iter().product::<f64>() - 0.1).abs() < 1.0e-10);
        assert!((atempo_factors(8.0).iter().product::<f64>() - 8.0).abs() < 1.0e-10);
    }

    #[test]
    fn split_and_linked_av_preserve_retimed_source_windows() {
        let mut value = project();
        value.sources.push(source());
        let clip_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value.clips[0].duration = 4.0;
        value
            .set_clip_source_time(&clip_id, SourceTimeMode::Forward, 2.0)
            .unwrap();
        let right_id = value.split_clip(&clip_id, 2.0).unwrap();
        let left = value.clips.iter().find(|clip| clip.id == clip_id).unwrap();
        let right = value.clips.iter().find(|clip| clip.id == right_id).unwrap();
        assert_eq!((left.source_start, right.source_start), (0.0, 4.0));
        assert_eq!(left.duration * left.playback_rate, 4.0);
        assert_eq!(right.duration * right.playback_rate, 4.0);
        value.validate().unwrap();

        value.remove_clip(&clip_id).unwrap();
        value.remove_clip(&right_id).unwrap();
        let reverse_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value
            .clips
            .iter_mut()
            .find(|clip| clip.id == reverse_id)
            .unwrap()
            .duration = 4.0;
        value
            .set_clip_source_time(&reverse_id, SourceTimeMode::Reverse, 2.0)
            .unwrap();
        let reverse_right = value.split_clip(&reverse_id, 2.0).unwrap();
        let reverse_left = value
            .clips
            .iter()
            .find(|clip| clip.id == reverse_id)
            .unwrap();
        let reverse_tail = value
            .clips
            .iter()
            .find(|clip| clip.id == reverse_right)
            .unwrap();
        assert_eq!(
            (reverse_left.source_start, reverse_tail.source_start),
            (4.0, 0.0)
        );
        value.validate().unwrap();

        let mut linked = project();
        linked.sources.push(source());
        let (video_id, audio_id) = linked.add_linked_av_clip("source-1", 0.0).unwrap();
        linked
            .set_clip_source_time(&video_id, SourceTimeMode::Reverse, 0.5)
            .unwrap();
        let video = linked
            .clips
            .iter()
            .find(|clip| clip.id == video_id)
            .unwrap();
        let audio = linked
            .clips
            .iter()
            .find(|clip| clip.id == audio_id)
            .unwrap();
        assert_eq!(video.source_time_mode, audio.source_time_mode);
        assert_eq!(video.playback_rate, audio.playback_rate);
        linked.validate().unwrap();
    }

    #[test]
    fn insert_ripple_splits_crossing_clips_and_moves_markers_and_linked_audio() {
        let mut value = project();
        value.sources.push(source());
        let (video_id, audio_id) = value.add_linked_av_clip("source-1", 0.0).unwrap();
        let marker_id = value.add_marker(7.0, "later cue").unwrap();
        let inserted = value
            .insert_clip_from_range("source-1", ClipKind::Video, 4.0, 0.0, 2.0, None)
            .unwrap();
        assert_eq!(value.duration, 14.0);
        let left = value.clips.iter().find(|clip| clip.id == video_id).unwrap();
        let right = value
            .clips
            .iter()
            .find(|clip| clip.id != video_id && clip.kind == ClipKind::Video)
            .unwrap();
        let audio_right = value
            .clips
            .iter()
            .find(|clip| clip.id != audio_id && clip.kind == ClipKind::Audio)
            .unwrap();
        assert_eq!(left.duration, 4.0);
        assert_eq!(right.timeline_start, 6.0);
        assert_eq!(audio_right.timeline_start, 6.0);
        assert_eq!(right.link_group_id, audio_right.link_group_id);
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == inserted)
                .unwrap()
                .timeline_start,
            4.0
        );
        assert_eq!(
            value
                .markers
                .iter()
                .find(|marker| marker.id == marker_id)
                .unwrap()
                .time,
            9.0
        );
        value.validate().unwrap();
    }

    #[test]
    fn failed_insert_on_locked_crossing_track_does_not_change_project() {
        let mut value = project();
        value.sources.push(source());
        let clip_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let track_id = value
            .clips
            .iter()
            .find(|clip| clip.id == clip_id)
            .unwrap()
            .track_id
            .clone()
            .unwrap();
        value
            .tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .unwrap()
            .locked = true;
        let before = value.clone();
        assert!(value
            .insert_clip_from_range("source-1", ClipKind::Video, 4.0, 0.0, 2.0, None)
            .is_err());
        assert_eq!(value, before);
    }

    #[test]
    fn overwrite_replaces_only_destination_interval_and_preserves_source_continuity() {
        let mut value = project();
        value.sources.push(source());
        let original = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value
            .clips
            .iter_mut()
            .find(|clip| clip.id == original)
            .unwrap()
            .duration = 10.0;
        let inserted = value
            .overwrite_clip_from_range("source-1", ClipKind::Video, 4.0, 2.0, 5.0, None)
            .unwrap();
        let left = value.clips.iter().find(|clip| clip.id == original).unwrap();
        let replacement = value.clips.iter().find(|clip| clip.id == inserted).unwrap();
        let right = value
            .clips
            .iter()
            .filter(|clip| {
                clip.kind == ClipKind::Video && clip.id != original && clip.id != inserted
            })
            .next()
            .unwrap();
        assert_eq!(
            (left.timeline_start, left.duration, left.source_start),
            (0.0, 4.0, 0.0)
        );
        assert_eq!(
            (
                replacement.timeline_start,
                replacement.duration,
                replacement.source_start
            ),
            (4.0, 3.0, 2.0)
        );
        assert_eq!(
            (right.timeline_start, right.duration, right.source_start),
            (7.0, 3.0, 7.0)
        );
        value.validate().unwrap();

        let (linked_video, _linked_audio) = value.add_linked_av_clip("source-1", 0.0).unwrap();
        let before = value.clone();
        assert!(value
            .overwrite_clip_from_range("source-1", ClipKind::Video, 1.0, 0.0, 1.0, None)
            .is_err());
        assert_eq!(value, before);
        assert!(value.clips.iter().any(|clip| clip.id == linked_video));
    }

    #[test]
    fn still_and_generated_color_sources_use_composition_instance_duration() {
        let mut value = project();
        let temp = tempfile::tempdir().unwrap();
        let image = temp.path().join("still.png");
        fs::write(&image, b"image fixture bytes").unwrap();
        value.sources.push(MediaSource {
            id: "still".into(),
            path: image,
            display_name: "still.png".into(),
            has_video: true,
            still_image: true,
            duration: 0.0,
            width: Some(640),
            height: Some(360),
            ..Default::default()
        });
        let still_clip = value.add_clip("still", ClipKind::Video, 2.0).unwrap();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == still_clip)
                .unwrap()
                .duration,
            10.0
        );
        let solid_id = value.add_solid_color_source("20242C").unwrap();
        let solid_clip = value.add_clip(&solid_id, ClipKind::Video, 1.0).unwrap();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == solid_clip)
                .unwrap()
                .duration,
            11.0
        );
        value.validate().unwrap();

        let work = temp.path().join("work");
        fs::create_dir_all(&work).unwrap();
        let (args, _) = build_ffmpeg_args(&value, Path::new("out.mp4"), &work, Some(0.5)).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(args.windows(2).any(|pair| pair == ["-loop", "1"]));
        assert!(args.join(" ").contains("color=c=0x20242C"));
    }

    #[test]
    fn removing_unused_media_only_edits_project_references() {
        let mut value = project();
        let temp = tempfile::tempdir().unwrap();
        let retained_file = temp.path().join("used.mp4");
        let unused_file = temp.path().join("unused.mp4");
        fs::write(&retained_file, b"user media").unwrap();
        fs::write(&unused_file, b"user media").unwrap();
        let mut used = source();
        used.id = "used-source".into();
        used.path = retained_file.clone();
        let mut unused = source();
        unused.id = "unused-source".into();
        unused.path = unused_file.clone();
        value.sources.extend([used, unused]);
        value.add_clip("used-source", ClipKind::Video, 0.0).unwrap();

        assert_eq!(value.remove_unused_sources(), 1);
        assert_eq!(value.sources.len(), 1);
        assert_eq!(value.sources[0].id, "used-source");
        assert!(retained_file.exists());
        assert!(unused_file.exists());
        assert_eq!(value.remove_unused_sources(), 0);
    }

    #[test]
    fn locked_tracks_block_timeline_edits_and_markers_persist() {
        let mut value = project();
        let mut media = source();
        media.duration = 4.0;
        let source_id = media.id.clone();
        value.sources.push(media);
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let track_id = value.clips[0].track_id.clone().unwrap();
        value
            .tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .unwrap()
            .locked = true;
        assert!(value.move_clip_to(&clip_id, 1.0).is_err());
        assert!(value.split_clip(&clip_id, 2.0).is_err());
        let marker_id = value.add_marker(1.5, "Beat").unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("markers.sonocomp");
        value.save(&path).unwrap();
        let reopened = CompositionProject::load(&path).unwrap();
        assert!(
            reopened
                .tracks
                .iter()
                .find(|track| track.id == track_id)
                .unwrap()
                .locked
        );
        assert_eq!(reopened.markers[0].id, marker_id);
        assert_eq!(reopened.markers[0].label, "Beat");
    }

    #[test]
    fn ripple_delete_keeps_linked_media_synchronized_and_is_atomic_on_lock_failure() {
        let mut value = project();
        value.duration = 20.0;
        let media = source();
        let source_id = media.id.clone();
        value.sources.push(media);
        let (first_video, first_audio) = value.add_linked_av_clip(&source_id, 0.0).unwrap();
        for clip in &mut value.clips {
            if clip.id == first_video || clip.id == first_audio {
                clip.duration = 4.0;
            }
        }
        let (later_video, later_audio) = value.add_linked_av_clip(&source_id, 8.0).unwrap();
        for clip in &mut value.clips {
            if clip.id == later_video || clip.id == later_audio {
                clip.duration = 4.0;
            }
        }
        let audio_track = value
            .clips
            .iter()
            .find(|clip| clip.id == later_audio)
            .unwrap()
            .track_id
            .clone()
            .unwrap();
        value
            .tracks
            .iter_mut()
            .find(|track| track.id == audio_track)
            .unwrap()
            .locked = true;
        let before = value.clone();
        assert!(value.ripple_delete_clip(&first_video).is_err());
        assert_eq!(
            value, before,
            "failed ripple edits must not partially shift clips"
        );

        value
            .tracks
            .iter_mut()
            .find(|track| track.id == audio_track)
            .unwrap()
            .locked = false;
        value.ripple_delete_clip(&first_video).unwrap();
        assert!(!value.clips.iter().any(|clip| clip.id == first_audio));
        for id in [later_video.as_str(), later_audio.as_str()] {
            assert_eq!(
                value
                    .clips
                    .iter()
                    .find(|clip| clip.id == id)
                    .unwrap()
                    .timeline_start,
                4.0
            );
        }
        value.validate().unwrap();
    }

    #[test]
    fn ripple_delete_trims_other_clips_crossing_the_removed_interval() {
        let mut value = project();
        value.sources.push(source());
        let selected = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let crossing = value.add_clip("source-1", ClipKind::Video, 2.0).unwrap();
        value
            .clips
            .iter_mut()
            .find(|clip| clip.id == selected)
            .unwrap()
            .duration = 4.0;
        value
            .clips
            .iter_mut()
            .find(|clip| clip.id == crossing)
            .unwrap()
            .duration = 4.0;
        value.ripple_delete_clip(&selected).unwrap();
        let remaining = value
            .clips
            .iter()
            .find(|clip| clip.source_start == 2.0 && clip.timeline_start == 0.0)
            .unwrap();
        assert_eq!(remaining.timeline_start, 0.0);
        assert_eq!(remaining.source_start, 2.0);
        assert_eq!(remaining.duration, 2.0);
        value.validate().unwrap();
    }

    #[test]
    fn new_video_clips_default_to_fitting_inside_the_composition_canvas() {
        let mut value = project();
        value.width = 1080;
        value.height = 1920;
        value.sources.push(source());
        let id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let clip = value.clips.iter().find(|clip| clip.id == id).unwrap();
        assert!((clip.scale_x - 0.5625).abs() < f64::EPSILON);
        assert_eq!(clip.scale_x, clip.scale_y);
    }

    #[test]
    fn position_keyframes_generate_spatial_interpolation_with_easing_not_time_stretch() {
        let keys = vec![
            PositionKeyframe {
                time: 0.0,
                x: 0.0,
                easing_to_next: Easing::EaseInOut,
            },
            PositionKeyframe {
                time: 2.0,
                x: 100.0,
                easing_to_next: Easing::Linear,
            },
        ];
        let expression = position_expr(0.0, &keys);
        assert!(expression.contains("(t-0.000000)/2.000000"));
        assert!(expression.contains("3-2*"));
    }

    #[test]
    fn scalar_animation_has_defined_boundaries_easing_and_hold_behavior() {
        let keys = vec![
            ScalarKeyframe {
                time: 1.0,
                value: 10.0,
                easing_to_next: Easing::Hold,
            },
            ScalarKeyframe {
                time: 3.0,
                value: 20.0,
                easing_to_next: Easing::Linear,
            },
        ];
        assert_eq!(evaluate_keyframes(&keys, 0.0, 99.0), 10.0);
        assert_eq!(evaluate_keyframes(&keys, 2.5, 99.0), 10.0);
        assert_eq!(evaluate_keyframes(&keys, 4.0, 99.0), 20.0);
        let mut eased = keys;
        eased[0].easing_to_next = Easing::EaseInOut;
        assert_eq!(evaluate_keyframes(&eased, 2.0, 0.0), 15.0);
    }

    #[test]
    fn track_model_assigns_compatible_tracks_and_rejects_cross_type_placement() {
        let mut value = project();
        value.sources.push(source());
        let video_id = value.add_track(TrackKind::Visual, "Overlay").unwrap();
        let audio_id = value.add_track(TrackKind::Audio, "Voiceover").unwrap();
        let visual = value
            .add_clip_to_track("source-1", ClipKind::Video, 0.0, Some(&video_id))
            .unwrap();
        let audio = value
            .add_clip_to_track("source-1", ClipKind::Audio, 0.0, Some(&audio_id))
            .unwrap();
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == visual)
                .unwrap()
                .track_id
                .as_deref(),
            Some(video_id.as_str())
        );
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == audio)
                .unwrap()
                .track_id
                .as_deref(),
            Some(audio_id.as_str())
        );
        value.clips[0].track_id = Some(audio_id);
        assert!(value.validate().is_err());
    }

    #[test]
    fn schema_one_compositions_migrate_tracks_and_x_keyframes_on_load() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy.sonocomp");
        let mut value = project();
        value.sources.push(source());
        let id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let clip = value.clips.iter_mut().find(|clip| clip.id == id).unwrap();
        clip.position_keyframes.push(PositionKeyframe {
            time: 1.0,
            x: 40.0,
            easing_to_next: Easing::Linear,
        });
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(1);
        json.as_object_mut().unwrap().remove("tracks");
        let clip = &mut json["clips"][0];
        clip.as_object_mut().unwrap().remove("track_id");
        clip.as_object_mut().unwrap().remove("animated_properties");
        clip.as_object_mut().unwrap().remove("enabled");
        clip.as_object_mut().unwrap().remove("rotation_degrees");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(migrated.clips[0].track_id.as_deref(), Some("visual-main"));
        assert_eq!(
            migrated.animated_value(&migrated.clips[0], AnimatedProperty::X, 1.0),
            40.0
        );
    }

    #[test]
    fn schema_two_migrates_missing_links_locks_markers_and_still_fields() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-two.sonocomp");
        let mut value = project();
        value.sources.push(source());
        value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(2);
        json.as_object_mut().unwrap().remove("markers");
        json["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("still_image");
        json["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("solid_color");
        json["tracks"][0].as_object_mut().unwrap().remove("locked");
        json["clips"][0]
            .as_object_mut()
            .unwrap()
            .remove("link_group_id");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(migrated.markers.is_empty());
        assert!(!migrated.sources[0].still_image);
        assert!(migrated.sources[0].solid_color.is_none());
        assert!(!migrated.tracks[0].locked);
        assert!(migrated.clips[0].link_group_id.is_none());
    }

    #[test]
    fn schema_six_migrates_through_later_composition_schemas() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-six.sonocomp");
        let mut json = serde_json::to_value(project()).unwrap();
        json["schema_version"] = serde_json::json!(6);
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(COMPOSITION_SCHEMA_VERSION, 21);
    }

    #[test]
    fn schema_twelve_migrates_to_historical_balanced_export_quality() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-twelve.sonocomp");
        let mut json = serde_json::to_value(project()).unwrap();
        json["schema_version"] = serde_json::json!(12);
        json.as_object_mut().unwrap().remove("export_preset");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(migrated.export_preset, CompositionExportPreset::Balanced);
    }

    #[test]
    fn schema_thirteen_migrates_without_changing_existing_color_sources() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-thirteen.sonocomp");
        let mut value = project();
        let source_id = value.add_solid_color_source("123456").unwrap();
        value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(13);
        json["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("generated_shape");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(COMPOSITION_SCHEMA_VERSION, 21);
        assert!(migrated.sources[0].generated_shape.is_none());
        assert_eq!(migrated.sources[0].solid_color.as_deref(), Some("123456"));
    }

    #[test]
    fn schema_fourteen_defaults_transition_easing_to_linear() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-fourteen.sonocomp");
        let mut value = project();
        let red = value.add_solid_color_source("E02020").unwrap();
        let blue = value.add_solid_color_source("2040E0").unwrap();
        let from = value.add_clip(&red, ClipKind::Video, 0.0).unwrap();
        let to = value.add_clip(&blue, ClipKind::Video, 3.0).unwrap();
        value.clips[0].duration = 6.0;
        value
            .add_transition_between(&from, &to, TransitionKind::CrossDissolve, 1.0)
            .unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(14);
        json["transitions"][0]
            .as_object_mut()
            .unwrap()
            .remove("easing");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(migrated.transitions[0].easing, Easing::Linear);
    }

    #[test]
    fn schema_sixteen_path_only_proxy_is_preserved_but_not_trusted() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-sixteen.sonocomp");
        let mut value = project();
        let mut media = source();
        media.proxy_path = Some(PathBuf::from("input.proxy.mp4"));
        value.sources.push(media);
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(16);
        json["sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("proxy_metadata");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(
            migrated.sources[0].proxy_path.as_deref(),
            Some(temp.path().join("input.proxy.mp4").as_path())
        );
        assert_eq!(
            migrated.source_proxy_status("source-1"),
            Some(VideoProxyStatus::MetadataMissing)
        );
    }

    #[test]
    fn schema_nineteen_defaults_new_audio_track_gain_to_unity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-nineteen.sonocomp");
        let mut value = project();
        let source_id = value.add_solid_color_source("404040").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        let effect_id = value
            .add_visual_effect(&clip_id, VisualEffectKind::Brightness { amount: 0.0 })
            .unwrap();
        value
            .set_effect_keyframe(
                &clip_id,
                &effect_id,
                EffectParameter::Amount,
                1.0,
                0.25,
                Easing::EaseInOut,
            )
            .unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(19);
        for track in json["tracks"].as_array_mut().unwrap() {
            track.as_object_mut().unwrap().remove("gain");
        }
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(COMPOSITION_SCHEMA_VERSION, 21);
        assert!(migrated.tracks.iter().all(|track| track.gain == 1.0));
        assert_eq!(
            migrated.clips[0].effects[0].animated_parameters[&EffectParameter::Amount][0].value,
            0.25
        );
        let output = temp.path().join("saved-after-migration.sonocomp");
        migrated.save(&output).unwrap();
        let reopened = CompositionProject::load(&output).unwrap();
        assert_eq!(reopened, migrated);
    }

    #[test]
    fn schema_seventeen_migrates_without_changing_existing_clip_semantics() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-seventeen.sonocomp");
        let mut value = project();
        value.sources.push(source());
        let clip_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let original = value.clips[0].clone();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(17);
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(COMPOSITION_SCHEMA_VERSION, 21);
        assert_eq!(migrated.clips[0], original);
        assert_eq!(migrated.clips[0].id, clip_id);
    }

    #[test]
    fn schema_eighteen_effects_default_to_static_values() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-eighteen.sonocomp");
        let mut value = project();
        let source_id = value.add_solid_color_source("4080C0").unwrap();
        let clip_id = value.add_clip(&source_id, ClipKind::Video, 0.0).unwrap();
        value
            .add_visual_effect(&clip_id, VisualEffectKind::Brightness { amount: 0.2 })
            .unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(18);
        for clip in json["clips"].as_array_mut().unwrap() {
            for effect in clip["effects"].as_array_mut().unwrap() {
                effect
                    .as_object_mut()
                    .unwrap()
                    .remove("animated_parameters");
            }
        }
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        let effect = &migrated.clips[0].effects[0];
        assert!(effect.animated_parameters.is_empty());
        assert!(matches!(effect.kind, VisualEffectKind::Brightness { amount } if amount == 0.2));
    }

    #[test]
    fn proxy_status_tracks_source_and_proxy_file_identity() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("source.mp4");
        let proxy_path = temp.path().join("source.proxy.mp4");
        fs::write(&original, b"original revision one").unwrap();
        fs::write(&proxy_path, b"generated proxy bytes").unwrap();
        let mut media = source();
        media.id = "proxy-source".into();
        media.path = original.clone();
        media.proxy_path = Some(proxy_path.clone());
        media.proxy_metadata = Some(VideoProxyMetadata {
            source_id: media.id.clone(),
            source_signature: media_file_signature(&original).unwrap(),
            proxy_signature: media_file_signature(&proxy_path).unwrap(),
            width: 960,
            height: 540,
            duration: media.duration,
            video_codec: "h264".into(),
            generated_by_renderer: true,
        });
        let mut value = project();
        value.sources.push(media);
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::Ready)
        );

        fs::write(&original, b"a changed original revision").unwrap();
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::Stale)
        );
        fs::write(&original, b"original revision one").unwrap();
        fs::remove_file(&original).unwrap();
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::ReadyOriginalOffline)
        );
        fs::write(&proxy_path, b"proxy was modified outside the editor").unwrap();
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::Stale)
        );
    }

    #[test]
    fn proxy_preview_falls_back_safely_and_export_always_names_original() {
        let temp = tempfile::tempdir().unwrap();
        let original = temp.path().join("original.mp4");
        let proxy_path = temp.path().join("proxy.mp4");
        fs::write(&original, b"original revision one").unwrap();
        fs::write(&proxy_path, b"proxy revision one").unwrap();
        let mut media = source();
        media.id = "proxy-source".into();
        media.path = original.clone();
        media.has_audio = false;
        media.proxy_path = Some(proxy_path.clone());
        media.proxy_metadata = Some(VideoProxyMetadata {
            source_id: media.id.clone(),
            source_signature: media_file_signature(&original).unwrap(),
            proxy_signature: media_file_signature(&proxy_path).unwrap(),
            width: 960,
            height: 540,
            duration: media.duration,
            video_codec: "h264".into(),
            generated_by_renderer: false,
        });
        let mut value = project();
        value.sources.push(media);
        value
            .add_clip("proxy-source", ClipKind::Video, 0.0)
            .unwrap();
        value.use_proxies_for_preview = true;

        fs::write(&original, b"a different source revision").unwrap();
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::Stale)
        );
        let (stale_preview_args, _) = build_ffmpeg_args(
            &value,
            Path::new("stale-preview.png"),
            temp.path(),
            Some(0.5),
        )
        .unwrap();
        assert!(stale_preview_args.contains(&original.as_os_str().to_owned()));
        assert!(!stale_preview_args.contains(&proxy_path.as_os_str().to_owned()));

        let (export_args, _) =
            build_ffmpeg_args(&value, Path::new("final.mp4"), temp.path(), None).unwrap();
        assert!(export_args.contains(&original.as_os_str().to_owned()));
        assert!(!export_args.contains(&proxy_path.as_os_str().to_owned()));

        fs::remove_file(&original).unwrap();
        assert_eq!(
            value.source_proxy_status("proxy-source"),
            Some(VideoProxyStatus::ReadyOriginalOffline)
        );
        let (offline_preview_args, _) = build_ffmpeg_args(
            &value,
            Path::new("offline-original-preview.png"),
            temp.path(),
            Some(0.5),
        )
        .unwrap();
        assert!(offline_preview_args.contains(&proxy_path.as_os_str().to_owned()));
        assert!(!offline_preview_args.contains(&original.as_os_str().to_owned()));
    }

    #[test]
    fn portable_template_rebinds_sources_scales_responsively_and_keeps_native_editable_objects() {
        let mut authored = CompositionProject::new();
        authored.width = 1920;
        authored.height = 1080;
        authored.fps = 30;
        authored.duration = 3.0;
        authored.sources.push(source());
        let base = authored.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let incoming = authored.add_clip("source-1", ClipKind::Video, 1.0).unwrap();
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == base)
                .unwrap();
            clip.duration = 2.0;
            clip.scale_x = 1.0;
            clip.scale_y = 1.0;
            clip.mask = Some(VisualMask {
                shape: VisualMaskShape::Ellipse,
                x: 0.1,
                y: 0.1,
                width: 0.8,
                height: 0.8,
                feather: 0.02,
                inverted: false,
            });
            clip.animated_properties.insert(
                AnimatedProperty::Opacity,
                vec![
                    ScalarKeyframe {
                        time: 0.0,
                        value: 0.0,
                        easing_to_next: Easing::EaseOut,
                    },
                    ScalarKeyframe {
                        time: 0.5,
                        value: 1.0,
                        easing_to_next: Easing::Linear,
                    },
                ],
            );
        }
        authored
            .add_visual_effect(&base, VisualEffectKind::GaussianBlur { sigma: 8.0 })
            .unwrap();
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == incoming)
                .unwrap();
            clip.timeline_start = 1.0;
            clip.duration = 2.0;
            clip.source_start = 1.0;
            clip.z_order = 1;
        }
        authored
            .add_transition_between(&base, &incoming, TransitionKind::CrossDissolve, 1.0)
            .unwrap();
        let title = authored
            .add_text_clip("{{param:title|Original title}}", 0.0, 3.0)
            .unwrap();
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == title)
                .unwrap();
            clip.x = 960.0;
            clip.y = 900.0;
            clip.font_size = 96;
            clip.outline_width = 8.0;
            clip.text_background_enabled = true;
            clip.text_background_opacity = 0.4;
        }
        authored.validate().unwrap();

        let template = CompositionTemplate::from_project(&authored, "Reusable edit").unwrap();
        assert_eq!(template.source_slots.len(), 1);
        assert_eq!(
            template.text_parameters,
            vec![TemplateTextParameter {
                name: "title".into(),
                default_value: "Original title".into(),
            }]
        );
        assert!(template
            .clips
            .iter()
            .all(|clip| clip.source_id.as_deref() != Some("source-1")));

        let temp = tempfile::tempdir().unwrap();
        let template_path = temp.path().join("reusable.sonotemplate");
        template.save(&template_path).unwrap();
        let reopened_template = CompositionTemplate::load(&template_path).unwrap();
        assert_eq!(reopened_template, template);

        let mut target = CompositionProject::new();
        target.width = 1080;
        target.height = 1920;
        target.fps = 30;
        target.duration = 2.0;
        let mut replacement = source();
        replacement.id = "replacement-video".into();
        replacement.path = "other-ordinary-video.mp4".into();
        replacement.display_name = "Other ordinary video".into();
        replacement.has_audio = false;
        replacement.width = Some(1280);
        replacement.height = Some(720);
        target.sources.push(replacement);
        let slot = &reopened_template.source_slots[0];
        let bindings = HashMap::from([(slot.id.clone(), "replacement-video".into())]);
        let values = HashMap::from([("title".into(), "Second source / vertical layout".into())]);
        let inserted = reopened_template
            .apply_to_project(&mut target, &bindings, &values, true)
            .unwrap();
        assert_eq!(inserted.len(), reopened_template.clips.len());
        assert_eq!(target.width, 1080);
        assert_eq!(target.height, 1920);
        assert_eq!(target.duration, 3.0);
        assert_eq!(target.transitions.len(), 1);
        assert!(target
            .clips
            .iter()
            .all(|clip| clip.id != base && clip.id != incoming && clip.id != title));
        let rebound = target
            .clips
            .iter()
            .filter(|clip| clip.kind == ClipKind::Video)
            .collect::<Vec<_>>();
        assert_eq!(rebound.len(), 2);
        assert!(rebound
            .iter()
            .all(|clip| clip.source_id.as_deref() == Some("replacement-video")));
        let rebound_title = target
            .clips
            .iter()
            .find(|clip| clip.kind == ClipKind::Text)
            .unwrap();
        assert_eq!(rebound_title.text, "Second source / vertical layout");
        assert_eq!(rebound_title.x, 540.0);
        assert_eq!(rebound_title.y, 1600.0);
        assert_eq!(rebound_title.font_size, 54);
        assert_eq!(rebound_title.outline_width, 4.5);
        assert!(rebound.iter().any(|clip| clip.mask.is_some()
            && !clip.effects.is_empty()
            && clip.has_animation(AnimatedProperty::Opacity)));
        target.validate().unwrap();

        // Applied templates remain normal editable project state.
        let extra = target
            .add_text_clip("Additional editable layer", 0.25, 1.0)
            .unwrap();
        let project_path = temp.path().join("template-instance.sonocomp");
        target.save(&project_path).unwrap();
        let reopened = CompositionProject::load(&project_path).unwrap();
        assert!(reopened.clips.iter().any(|clip| clip.id == extra));
        assert!(reopened
            .clips
            .iter()
            .any(|clip| clip.kind == ClipKind::Text
                && clip.text == "Second source / vertical layout"));

        let before_missing_binding = target.clone();
        assert!(reopened_template
            .apply_to_project(&mut target, &HashMap::new(), &HashMap::new(), true)
            .is_err());
        assert_eq!(target, before_missing_binding);
    }

    #[test]
    fn portable_template_rejects_paths_future_versions_and_nonportable_fonts() {
        let mut authored = project();
        authored.sources.push(source());
        let text = authored.add_text_clip("Text", 0.0, 1.0).unwrap();
        authored
            .clips
            .iter_mut()
            .find(|clip| clip.id == text)
            .unwrap()
            .font_path = Some(PathBuf::from("C:/machine/private/font.ttf"));
        assert!(CompositionTemplate::from_project(&authored, "No absolute fonts").is_err());

        authored
            .clips
            .iter_mut()
            .find(|clip| clip.id == text)
            .unwrap()
            .font_path = None;
        let template = CompositionTemplate::from_project(&authored, "Safe").unwrap();
        let mut value = serde_json::to_value(template).unwrap();
        value["schema_version"] = serde_json::json!(COMPOSITION_TEMPLATE_SCHEMA_VERSION + 1);
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("future.sonotemplate");
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(CompositionTemplate::load(&path).is_err());
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    #[test]
    fn generated_shape_sources_validate_and_compile_as_transparent_geometry() {
        let mut value = project();
        let ellipse = value
            .add_shape_source(GeneratedShapeKind::Ellipse, "F5C542")
            .unwrap();
        let rectangle = value
            .add_shape_source(GeneratedShapeKind::Rectangle, "32A0E0")
            .unwrap();
        value.add_clip(&ellipse, ClipKind::Video, 0.0).unwrap();
        value.add_clip(&rectangle, ClipKind::Video, 0.0).unwrap();
        value.validate().unwrap();

        let temp = tempfile::tempdir().unwrap();
        let (args, _) =
            build_ffmpeg_args(&value, &temp.path().join("shapes.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let graph = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| &args[index + 1])
            .unwrap();
        assert!(graph.contains("pow((X-"));
        assert!(graph.contains("gte(X,"));

        value.sources[0].generated_shape.as_mut().unwrap().width = f64::NAN;
        assert!(value.validate().is_err());
    }

    #[test]
    fn final_export_arguments_follow_the_project_quality_preset() {
        let temp = tempfile::tempdir().unwrap();
        let mut project = CompositionProject::new();
        project.duration = 1.0;
        let source = project.add_solid_color_source("123456").unwrap();
        project.add_clip(&source, ClipKind::Video, 0.0).unwrap();
        project.export_preset = CompositionExportPreset::HighQuality;
        let project_path = temp.path().join("quality.sonocomp");
        project.save(&project_path).unwrap();
        assert_eq!(
            CompositionProject::load(&project_path)
                .unwrap()
                .export_preset,
            CompositionExportPreset::HighQuality
        );

        let (args, _) =
            build_ffmpeg_args(&project, &temp.path().join("out.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let crf = args.iter().position(|arg| arg == "-crf").unwrap();
        let preset = args.iter().position(|arg| arg == "-preset").unwrap();
        assert_eq!(args[crf + 1], "16");
        assert_eq!(args[preset + 1], "slow");
    }

    #[test]
    fn schema_seven_migrates_without_grouping_existing_clips() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-seven.sonocomp");
        let mut value = project();
        value.sources.push(source());
        value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(7);
        for clip in json["clips"].as_array_mut().unwrap() {
            clip.as_object_mut().unwrap().remove("group_id");
        }
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(migrated.clips.iter().all(|clip| clip.group_id.is_none()));
    }

    #[test]
    fn schema_eight_migrates_with_text_background_defaults_disabled() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-eight.sonocomp");
        let mut value = project();
        let clip_id = value.add_text_clip("Legacy caption", 0.0, 3.0).unwrap();
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(8);
        let clip = json["clips"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|clip| clip["id"] == clip_id)
            .unwrap();
        for field in [
            "text_background_enabled",
            "text_background_color",
            "text_background_opacity",
            "text_background_padding",
        ] {
            clip.as_object_mut().unwrap().remove(field);
        }
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        let clip = &migrated.clips[0];
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(!clip.text_background_enabled);
        assert_eq!(clip.text_background_color, "000000");
        assert_eq!(clip.text_background_opacity, 0.65);
        assert_eq!(clip.text_background_padding, 12);
        assert!(migrated.export_range.is_none());
    }

    #[test]
    fn schema_nine_migrates_with_full_duration_export_default() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-nine.sonocomp");
        let mut json = serde_json::to_value(project()).unwrap();
        json["schema_version"] = serde_json::json!(9);
        json.as_object_mut().unwrap().remove("export_range");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(migrated.export_range.is_none());
        assert_eq!(migrated.export_duration(), migrated.duration);
    }

    #[test]
    fn schema_ten_migrates_with_local_title_presets() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-ten.sonocomp");
        let mut json = serde_json::to_value(project()).unwrap();
        json["schema_version"] = serde_json::json!(10);
        json.as_object_mut().unwrap().remove("title_presets");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert_eq!(migrated.title_presets.len(), 4);
        assert!(migrated.title_presets.iter().all(|preset| preset.built_in));
    }

    #[test]
    fn schema_twenty_migration_preserves_user_presets_and_adds_short_form_style() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-twenty.sonocomp");
        let mut old = project();
        old.schema_version = 20;
        old.title_presets
            .retain(|preset| preset.id != "builtin-short-form-caption");
        old.title_presets.push(TitlePreset {
            id: "user-caption-style".into(),
            name: "My caption style".into(),
            built_in: false,
            ..Default::default()
        });
        fs::write(&path, serde_json::to_vec(&old).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(migrated
            .title_presets
            .iter()
            .any(|preset| preset.id == "user-caption-style"));
        assert!(migrated
            .title_presets
            .iter()
            .any(|preset| preset.id == "builtin-short-form-caption" && preset.built_in));
    }

    #[test]
    fn schema_eleven_migrates_with_composition_identity_and_source_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-eleven.sonocomp");
        let mut json = serde_json::to_value(project()).unwrap();
        json["schema_version"] = serde_json::json!(11);
        json.as_object_mut().unwrap().remove("composition_id");
        json["sources"] = serde_json::json!([{
            "id": "old-source",
            "path": "old.mp4",
            "display_name": "old.mp4",
            "has_video": true,
            "has_audio": false,
            "duration": 3.0
        }]);
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(!migrated.composition_id.is_empty());
        assert!(migrated.sources[0].nested_composition_path.is_none());
    }

    #[test]
    fn nested_composition_references_reopen_and_reject_deep_cycles_atomically() {
        let temp = tempfile::tempdir().unwrap();
        let parent_path = temp.path().join("parent.sonocomp");
        let child_path = temp.path().join("child.sonocomp");
        let grandchild_path = temp.path().join("grandchild.sonocomp");
        let mut parent = CompositionProject::new();
        parent.name = "Parent".into();
        parent.duration = 8.0;
        parent.save(&parent_path).unwrap();

        let mut grandchild = CompositionProject::new();
        grandchild.name = "Grandchild".into();
        grandchild
            .add_nested_composition_source(&parent_path)
            .unwrap();
        grandchild.save(&grandchild_path).unwrap();

        let mut child = CompositionProject::new();
        child.name = "Child".into();
        child.duration = 4.0;
        child
            .add_nested_composition_source(&grandchild_path)
            .unwrap();
        child.save(&child_path).unwrap();

        let before = parent.clone();
        assert!(parent.add_nested_composition_source(&child_path).is_err());
        assert_eq!(parent, before, "cycle rejection must not mutate the parent");

        let mut acyclic_child = CompositionProject::new();
        acyclic_child.name = "Acyclic Child".into();
        acyclic_child
            .save(temp.path().join("acyclic-child.sonocomp"))
            .unwrap();
        let child_path = temp.path().join("acyclic-child.sonocomp");
        let nested_id = parent.add_nested_composition_source(&child_path).unwrap();
        parent.add_clip(&nested_id, ClipKind::Video, 0.0).unwrap();
        parent.save(&parent_path).unwrap();
        let stored: Value = serde_json::from_slice(&fs::read(&parent_path).unwrap()).unwrap();
        assert_eq!(
            stored["sources"][0]["nested_composition_path"],
            "acyclic-child.sonocomp"
        );
        let reopened = CompositionProject::load(&parent_path).unwrap();
        let nested = reopened
            .sources
            .iter()
            .find(|source| source.id == nested_id)
            .unwrap();
        assert_eq!(
            nested.nested_composition_path.as_deref(),
            Some(child_path.as_path())
        );
        assert_eq!(
            reopened.clips[0].source_id.as_deref(),
            Some(nested_id.as_str())
        );
    }

    #[test]
    fn schema_three_migrates_legacy_blur_without_changing_its_semantics() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("schema-three.sonocomp");
        let mut value = project();
        value.sources.push(source());
        let clip_id = value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        let clip = value
            .clips
            .iter_mut()
            .find(|clip| clip.id == clip_id)
            .unwrap();
        clip.blur = 7.5;
        clip.animated_properties.insert(
            AnimatedProperty::Blur,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 7.5,
                    easing_to_next: Easing::Linear,
                },
                ScalarKeyframe {
                    time: 1.0,
                    value: 2.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        let mut json = serde_json::to_value(value).unwrap();
        json["schema_version"] = serde_json::json!(3);
        json["clips"][0].as_object_mut().unwrap().remove("effects");
        fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();

        let migrated = CompositionProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, COMPOSITION_SCHEMA_VERSION);
        assert!(migrated.clips[0].effects.is_empty());
        assert_eq!(migrated.clips[0].blur, 7.5);
        assert_eq!(
            migrated.clips[0].animated_value(AnimatedProperty::Blur, 0.5),
            4.75
        );
    }

    #[test]
    fn generated_filter_graph_splits_shared_source_and_mixes_audio_once_per_routed_clip() {
        let mut value = project();
        value.sources.push(source());
        let mut main = CompositionClip {
            id: "v1".into(),
            name: "Main".into(),
            kind: ClipKind::Video,
            source_id: Some("source-1".into()),
            duration: 8.0,
            ..Default::default()
        };
        main.scale_x = 0.32;
        main.scale_y = 0.32;
        main.blur = 14.0;
        let detail = CompositionClip {
            id: "v2".into(),
            name: "Detail".into(),
            kind: ClipKind::Video,
            source_id: Some("source-1".into()),
            timeline_start: 2.0,
            duration: 8.0,
            z_order: 1,
            ..Default::default()
        };
        let audio = CompositionClip {
            id: "a1".into(),
            name: "Audio".into(),
            kind: ClipKind::Audio,
            source_id: Some("source-1".into()),
            duration: 10.0,
            gain: 0.8,
            fade_in: 1.0,
            fade_out: 1.0,
            ..Default::default()
        };
        let text = CompositionClip {
            id: "t1".into(),
            name: "Caption".into(),
            kind: ClipKind::Text,
            source_id: None,
            text: "Hello".into(),
            duration: 5.0,
            ..Default::default()
        };
        let mut text = text;
        text.text_background_enabled = true;
        text.text_background_color = "183050".into();
        text.text_background_opacity = 0.7;
        text.text_background_padding = 18;
        value.clips = vec![main, detail, audio, text];
        value.validate().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let media_path = temp.path().join("input.mp4");
        fs::write(&media_path, b"filter graph fixture").unwrap();
        value.sources[0].path = media_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("split=2"));
        assert!(args.contains("gblur@blur_0=sigma=14.0000"));
        assert!(args.contains("amix=inputs=1"));
        assert!(args.contains("afade=t=in"));
        assert!(args.contains("drawtext="));
        assert!(args.contains("box=1:boxcolor=0x183050@0.700:boxborderw=18"));
        assert!(args.contains("-map [audio_mix]"));
    }

    #[test]
    fn preview_selects_the_requested_frame_without_output_seeking_past_it() {
        let mut value = project();
        value.sources.push(source());
        value.clips.push(CompositionClip {
            id: "v1".into(),
            name: "Video".into(),
            kind: ClipKind::Video,
            source_id: Some("source-1".into()),
            duration: 8.0,
            ..Default::default()
        });
        let temp = tempfile::tempdir().unwrap();
        let media_path = temp.path().join("input.mp4");
        fs::write(&media_path, b"preview graph fixture").unwrap();
        value.sources[0].path = media_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("preview.png"), temp.path(), Some(0.25)).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        let filter = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| &args[index + 1])
            .unwrap();
        assert!(filter.contains("select='eq(n,15)'"));
        assert!(!args.iter().any(|arg| arg == "-ss"));
    }

    #[test]
    fn audio_mute_solo_and_pan_are_applied_by_the_existing_composition_mixer() {
        let mut value = project();
        let audio_track = value
            .tracks
            .iter_mut()
            .find(|track| track.kind == TrackKind::Audio)
            .unwrap();
        audio_track.gain = 0.5;
        let audio_track_id = audio_track.id.clone();
        value.sources.push(source());
        let background = CompositionClip {
            id: "background-audio".into(),
            name: "Background".into(),
            kind: ClipKind::Audio,
            source_id: Some("source-1".into()),
            duration: 8.0,
            track_id: Some(audio_track_id.clone()),
            ..Default::default()
        };
        let voiceover = CompositionClip {
            id: "voiceover".into(),
            name: "Voiceover".into(),
            kind: ClipKind::Audio,
            source_id: Some("source-1".into()),
            duration: 8.0,
            track_id: Some(audio_track_id),
            solo: true,
            pan: 0.35,
            animated_properties: BTreeMap::from([(
                AnimatedProperty::Gain,
                vec![
                    ScalarKeyframe {
                        time: 1.0,
                        value: 0.25,
                        easing_to_next: Easing::EaseIn,
                    },
                    ScalarKeyframe {
                        time: 3.0,
                        value: 0.75,
                        easing_to_next: Easing::Linear,
                    },
                ],
            )]),
            ..Default::default()
        };
        value.clips = vec![background, voiceover];
        value.validate().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let media_path = temp.path().join("input.mp4");
        fs::write(&media_path, b"filter graph fixture").unwrap();
        value.sources[0].path = media_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("volume='0':eval=frame"));
        assert!(args.contains("eval=frame"));
        assert!(args.contains("0.250000"));
        assert!(args.contains("0.750000"));
        assert!(
            args.contains("*0.500000"),
            "track gain missing from audio graph: {args}"
        );
        assert!(args.contains("pan=stereo|c0=0.650000*c0|c1=1.000000*c1"));
        assert!(args.contains("amix=inputs=2"));
    }

    #[test]
    fn audio_crossfade_uses_overlapping_clip_fades_and_rejects_ambiguous_pairs() {
        let mut value = project();
        value.sources.push(source());
        let outgoing = value.add_clip("source-1", ClipKind::Audio, 0.0).unwrap();
        let incoming = value.add_clip("source-1", ClipKind::Audio, 4.0).unwrap();

        assert_eq!(
            value.add_audio_crossfade(&outgoing, &incoming).unwrap(),
            8.0
        );
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == outgoing)
                .unwrap()
                .fade_out,
            8.0
        );
        assert_eq!(
            value
                .clips
                .iter()
                .find(|clip| clip.id == incoming)
                .unwrap()
                .fade_in,
            8.0
        );
        let temp = tempfile::tempdir().unwrap();
        let media_path = temp.path().join("audio.mp4");
        fs::write(&media_path, b"crossfade graph fixture").unwrap();
        value.sources[0].path = media_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("output.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        assert!(args.contains("afade=t=out"));
        assert!(args.contains("afade=t=in"));

        let mut split_track = project();
        split_track.sources.push(source());
        let outgoing = split_track
            .add_clip("source-1", ClipKind::Audio, 0.0)
            .unwrap();
        let other_track = split_track
            .add_track(TrackKind::Audio, "Second audio")
            .unwrap();
        let incoming = split_track
            .add_clip_to_track("source-1", ClipKind::Audio, 4.0, Some(&other_track))
            .unwrap();
        let before = split_track.clone();
        assert!(split_track
            .add_audio_crossfade(&outgoing, &incoming)
            .is_err());
        assert_eq!(split_track, before);
    }

    #[test]
    fn export_ranges_are_validated_persisted_and_applied_to_video_and_audio_graphs() {
        let mut value = project();
        value.sources.push(source());
        value.add_clip("source-1", ClipKind::Video, 0.0).unwrap();
        value.add_clip("source-1", ClipKind::Audio, 0.0).unwrap();
        value.set_export_range(2.0, 5.0).unwrap();
        assert_eq!(value.export_duration(), 3.0);

        let temp = tempfile::tempdir().unwrap();
        let media_path = temp.path().join("range-source.mp4");
        fs::write(&media_path, b"export range graph fixture").unwrap();
        value.sources[0].path = media_path;
        let (args, _) =
            build_ffmpeg_args(&value, Path::new("range.mp4"), temp.path(), None).unwrap();
        let args = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>();
        let filter = args
            .iter()
            .position(|arg| arg == "-filter_complex")
            .map(|index| &args[index + 1])
            .unwrap();
        assert!(filter.contains("trim=start=2.000000:end=5.000000"));
        assert!(filter.contains("atrim=start=2.000000:end=5.000000"));

        let path = temp.path().join("range.sonocomp");
        value.save(&path).unwrap();
        let reopened = CompositionProject::load(&path).unwrap();
        assert_eq!(reopened.export_range, value.export_range);

        let before = value.clone();
        assert!(value.set_export_range(5.0, 5.0).is_err());
        assert_eq!(value, before);
        value.clear_export_range();
        assert_eq!(value.export_duration(), value.duration);
    }

    #[test]
    fn project_local_title_presets_apply_save_reopen_and_remove_as_text_clips() {
        let mut value = project();
        let title_id = value
            .add_title_from_preset("builtin-lower-third", 2.0, 4.0)
            .unwrap();
        let title = value.clips.iter().find(|clip| clip.id == title_id).unwrap();
        assert_eq!(title.kind, ClipKind::Text);
        assert_eq!(title.name, "Lower third");
        assert_eq!(title.text, "Name\nSubtitle");
        assert!((title.x - f64::from(value.width) * 0.08).abs() < 1.0e-6);
        assert!(title.text_background_enabled);

        let title = value
            .clips
            .iter_mut()
            .find(|clip| clip.id == title_id)
            .unwrap();
        title.text = "My name".into();
        title.text_color = "48D1CC".into();
        let preset_id = value
            .save_title_preset_from_clip(&title_id, "Teal lower third")
            .unwrap();
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("titles.sonocomp");
        value.save(&path).unwrap();
        let mut reopened = CompositionProject::load(&path).unwrap();
        let preset = reopened
            .title_presets
            .iter()
            .find(|preset| preset.id == preset_id)
            .unwrap();
        assert_eq!(preset.sample_text, "My name");
        assert_eq!(preset.text_color, "48D1CC");

        let second_id = reopened
            .add_title_from_preset(&preset_id, 8.0, 3.0)
            .unwrap();
        let second = reopened
            .clips
            .iter()
            .find(|clip| clip.id == second_id)
            .unwrap();
        assert_eq!(second.text, "My name");
        assert_eq!(second.text_color, "48D1CC");
        assert!(reopened
            .save_title_preset_from_clip(&second_id, "teal LOWER THIRD")
            .is_err());
        assert!(reopened
            .remove_title_preset("builtin-centered-title")
            .is_err());
        reopened.remove_title_preset(&preset_id).unwrap();
        assert!(!reopened
            .title_presets
            .iter()
            .any(|preset| preset.id == preset_id));
        reopened.validate().unwrap();

        let mut small = CompositionProject::new();
        small.width = 640;
        small.height = 360;
        let small_title = small
            .add_title_from_preset("builtin-lower-third", 0.0, 2.0)
            .unwrap();
        let small_title = small
            .clips
            .iter()
            .find(|clip| clip.id == small_title)
            .unwrap();
        assert_eq!(small_title.font_size, 34);
        assert_eq!(small_title.y, 0.58 * 360.0);
        assert_eq!(small_title.text_background_padding, 11);
    }

    #[test]
    fn composition_save_reopen_keeps_paths_relative_and_unknown_fields() {
        let temp = tempfile::tempdir().unwrap();
        let media = temp.path().join("audio.wav");
        fs::write(&media, b"fixture").unwrap();
        let path = temp.path().join("project.sonocomp");
        let mut value = project();
        value
            .extra
            .insert("futureField".into(), serde_json::json!({"kept": true}));
        value.sources.push(MediaSource {
            path: media.clone(),
            duration: 2.0,
            has_audio: true,
            ..source()
        });
        let text = value.add_text_clip("Relative font", 0.0, 2.0).unwrap();
        let text = value.clips.iter_mut().find(|clip| clip.id == text).unwrap();
        let font_path = temp.path().join("fonts/display.otf");
        fs::create_dir_all(font_path.parent().unwrap()).unwrap();
        fs::write(&font_path, b"font fixture").unwrap();
        text.font_path = Some(font_path.clone());
        text.text_alignment = TextAlignment::Center;
        value.save(&path).unwrap();
        let stored: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(stored["sources"][0]["path"], "audio.wav");
        assert_eq!(stored["clips"][0]["font_path"], "fonts/display.otf");
        value.width = 1920;
        value.height = 1080;
        value.save(&path).unwrap();
        let reopened = CompositionProject::load(&path).unwrap();
        assert_eq!(reopened.width, 1920);
        assert_eq!(reopened.height, 1080);
        assert_eq!(reopened.sources[0].path, media);
        assert_eq!(
            reopened.clips[0].font_path.as_deref(),
            Some(font_path.as_path())
        );
        assert_eq!(reopened.clips[0].text_alignment, TextAlignment::Center);
        assert_eq!(reopened.extra["futureField"]["kept"], true);
    }

    #[test]
    fn future_schema_versions_are_rejected_without_rewriting_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("future.sonocomp");
        let mut value = serde_json::to_value(CompositionProject::new()).unwrap();
        value["schema_version"] = serde_json::json!(COMPOSITION_SCHEMA_VERSION + 1);
        let bytes = serde_json::to_vec(&value).unwrap();
        fs::write(&path, &bytes).unwrap();
        assert!(CompositionProject::load(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }

    #[test]
    #[ignore = "requires an already installed FFmpeg/FFprobe pair; does not download tools"]
    fn real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export() -> Result<()> {
        use anyhow::ensure;

        let tools = crate::ffmpeg::inspect_existing()?
            .context("this integration test requires the app-managed FFmpeg/FFprobe pair")?;
        let temp = tempfile::tempdir()?;
        let video_a = temp.path().join("ordinary-video-a.mp4");
        let video_b = temp.path().join("ordinary-video-b.mp4");
        let video_c = temp.path().join("ordinary-video-c.mp4");
        let audio_a = temp.path().join("ordinary-audio-a.wav");
        let audio_b = temp.path().join("ordinary-audio-b.wav");
        let camera_av = temp.path().join("ordinary-camera-av.mp4");
        let still_png = temp.path().join("ordinary-still.png");
        let still_jpeg = temp.path().join("ordinary-still.jpg");
        for (output_path, input, codec_args) in [
            (
                &video_a,
                "testsrc2=size=640x360:rate=30:duration=3",
                vec!["-c:v", "libx264", "-pix_fmt", "yuv420p"],
            ),
            (
                &video_b,
                "color=c=red:s=640x360:r=30:d=3",
                vec!["-c:v", "libx264", "-pix_fmt", "yuv420p"],
            ),
            (
                &video_c,
                "color=c=blue:s=640x360:r=30:d=3",
                vec!["-c:v", "libx264", "-pix_fmt", "yuv420p"],
            ),
            (
                &audio_a,
                "sine=frequency=440:sample_rate=48000:duration=3",
                vec!["-c:a", "pcm_s16le"],
            ),
            (
                &audio_b,
                "sine=frequency=660:sample_rate=48000:duration=3",
                vec!["-c:a", "pcm_s16le"],
            ),
        ] {
            let mut command = Command::new(&tools.ffmpeg);
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                ])
                .arg(input)
                .args(codec_args)
                .arg(output_path);
            let result = crate::export_control::ExportControl::default()
                .output(crate::export_control::hide_child_window(&mut command))?;
            ensure!(
                result.status.success(),
                "fixture media generation failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }

        let mut unity_track_gain = CompositionProject::new();
        unity_track_gain.duration = 3.0;
        let audio_source = unity_track_gain.add_source(&audio_a)?;
        unity_track_gain.add_clip(&audio_source, ClipKind::Audio, 0.0)?;
        let half_track_gain = {
            let mut value = unity_track_gain.clone();
            value
                .tracks
                .iter_mut()
                .find(|track| track.kind == TrackKind::Audio)
                .context("audio track missing from PCM gain fixture")?
                .gain = 0.5;
            value
        };
        let gain_work = temp.path().join("track-gain-work");
        fs::create_dir_all(&gain_work)?;
        let (unity_program, unity_args) =
            playback_audio_command(&unity_track_gain, 48_000, &gain_work)?;
        let unity_pcm = Command::new(unity_program).args(unity_args).output()?;
        ensure!(
            unity_pcm.status.success(),
            "unity-gain PCM fixture failed: {}",
            String::from_utf8_lossy(&unity_pcm.stderr)
        );
        let (half_program, half_args) =
            playback_audio_command(&half_track_gain, 48_000, &gain_work)?;
        let half_pcm = Command::new(half_program).args(half_args).output()?;
        ensure!(
            half_pcm.status.success(),
            "half-gain PCM fixture failed: {}",
            String::from_utf8_lossy(&half_pcm.stderr)
        );
        let pcm_rms = |bytes: &[u8]| {
            let samples = bytes.chunks_exact(4).map(|sample| {
                f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]) as f64
            });
            let (sum, count) = samples.fold((0.0, 0_u64), |(sum, count), sample| {
                (sum + sample * sample, count + 1)
            });
            (sum / count.max(1) as f64).sqrt()
        };
        let unity_rms = pcm_rms(&unity_pcm.stdout);
        let half_rms = pcm_rms(&half_pcm.stdout);
        ensure!(
            unity_rms > 0.001 && (half_rms / unity_rms - 0.5).abs() < 0.02,
            "audio-track gain did not scale decoded PCM by one half: unity={unity_rms:.5}, half={half_rms:.5}"
        );

        for (path, color) in [(&still_png, "orange"), (&still_jpeg, "purple")] {
            let mut command = Command::new(&tools.ffmpeg);
            command
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                ])
                .arg(format!("color=c={color}:s=320x180"))
                .args(["-frames:v", "1"])
                .arg(path);
            let result = crate::export_control::ExportControl::default()
                .output(crate::export_control::hide_child_window(&mut command))?;
            ensure!(
                result.status.success(),
                "still-image fixture generation failed: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let mut camera_command = Command::new(&tools.ffmpeg);
        camera_command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=320x180:rate=30:duration=3",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=880:sample_rate=48000:duration=3",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-shortest",
            ])
            .arg(&camera_av);
        let result = crate::export_control::ExportControl::default().output(
            crate::export_control::hide_child_window(&mut camera_command),
        )?;
        ensure!(
            result.status.success(),
            "linked A/V fixture generation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );

        // Exercise the full generated-proxy path with a source larger than
        // the editor proxy cap. The source and proxy stay ordinary standalone
        // media; no Sonolus runtime participates in this fixture.
        let proxy_source_path = temp.path().join("proxy-source-1280x720.mp4");
        let mut proxy_source_command = Command::new(&tools.ffmpeg);
        proxy_source_command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=1280x720:rate=30:duration=2",
                "-c:v",
                "libx264",
                "-crf",
                "18",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(&proxy_source_path);
        let result = crate::export_control::ExportControl::default().output(
            crate::export_control::hide_child_window(&mut proxy_source_command),
        )?;
        ensure!(
            result.status.success(),
            "large proxy source fixture generation failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let generated_proxy_path = temp.path().join("proxy-source.proxy.mp4");
        let generated_proxy = generate_video_proxy(
            &proxy_source_path,
            &generated_proxy_path,
            &crate::export_control::ExportControl::default(),
        )?;
        let proxy_probe = probe_media(&generated_proxy)?;
        ensure!(
            proxy_probe.width == Some(960)
                && proxy_probe.height == Some(540)
                && !proxy_probe.has_audio,
            "generated proxy must be video-only at the configured 960-pixel long edge"
        );
        let mut proxy_project = CompositionProject::new();
        proxy_project.width = 640;
        proxy_project.height = 360;
        proxy_project.fps = 30;
        proxy_project.duration = 2.0;
        proxy_project.export_preset = CompositionExportPreset::Draft;
        let proxy_source_id = proxy_project.add_source(&proxy_source_path)?;
        proxy_project.add_clip(&proxy_source_id, ClipKind::Video, 0.0)?;
        proxy_project.set_generated_source_proxy(&proxy_source_id, &generated_proxy)?;
        ensure!(
            proxy_project.source_proxy_status(&proxy_source_id) == Some(VideoProxyStatus::Ready),
            "newly generated proxy did not validate against the canonical source"
        );
        let proxy_project_path = temp.path().join("proxy-acceptance.sonocomp");
        proxy_project.use_proxies_for_preview = true;
        proxy_project.save(&proxy_project_path)?;
        let proxy_reopened = CompositionProject::load(&proxy_project_path)?;
        ensure!(
            proxy_reopened.use_proxies_for_preview
                && proxy_reopened.source_proxy_status(&proxy_source_id)
                    == Some(VideoProxyStatus::Ready),
            "proxy metadata and preview choice did not survive project save/reopen"
        );
        let (proxy_preview_args, _) = build_ffmpeg_args(
            &proxy_reopened,
            Path::new("proxy-preview.png"),
            temp.path(),
            Some(0.5),
        )?;
        let proxy_preview_args = proxy_preview_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let generated_proxy_string = generated_proxy.to_string_lossy().into_owned();
        ensure!(
            proxy_preview_args
                .iter()
                .any(|arg| {
                    Path::new(arg).canonicalize().ok().as_deref() == Some(generated_proxy.as_path())
                }),
            "proxy-enabled preview graph did not select the attached proxy: expected {generated_proxy_string:?}; status={:?}; args={proxy_preview_args:?}",
            proxy_reopened.source_proxy_status(&proxy_source_id)
        );
        let (final_export_args, _) = build_ffmpeg_args(
            &proxy_reopened,
            Path::new("proxy-final.mp4"),
            temp.path(),
            None,
        )?;
        let final_export_args = final_export_args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let canonical_proxy_source = proxy_source_path.canonicalize()?;
        ensure!(
            final_export_args.iter().any(|arg| {
                Path::new(arg).canonicalize().ok().as_deref()
                    == Some(canonical_proxy_source.as_path())
            }) && !final_export_args.iter().any(|arg| {
                Path::new(arg).canonicalize().ok().as_deref() == Some(generated_proxy.as_path())
            }),
            "final export graph selected proxy media instead of the original source"
        );
        let proxy_frame = preview_png(&proxy_reopened, 0.5, &Default::default())?;
        let mut original_preview_project = proxy_reopened.clone();
        original_preview_project.use_proxies_for_preview = false;
        let original_frame = preview_png(&original_preview_project, 0.5, &Default::default())?;
        let proxy_pixels = image::load_from_memory(&proxy_frame)?.to_rgba8();
        let original_pixels = image::load_from_memory(&original_frame)?.to_rgba8();
        let different_pixels = proxy_pixels
            .as_raw()
            .chunks_exact(4)
            .zip(original_pixels.as_raw().chunks_exact(4))
            .filter(|(proxy, original)| proxy != original)
            .count();
        ensure!(
            different_pixels > 100,
            "proxy and original preview frames unexpectedly decoded identically"
        );
        let validation_dir = std::env::current_dir()?.join("artifacts/nle-validation");
        fs::create_dir_all(&validation_dir)?;
        fs::write(
            validation_dir.join("proxy-preview-original.png"),
            &original_frame,
        )?;
        fs::write(
            validation_dir.join("proxy-preview-generated.png"),
            &proxy_frame,
        )?;
        let proxy_export = temp.path().join("proxy-mode-enabled-export.mp4");
        let original_export = temp.path().join("proxy-mode-disabled-export.mp4");
        render(&proxy_reopened, &proxy_export, &Default::default())?;
        render(
            &original_preview_project,
            &original_export,
            &Default::default(),
        )?;
        let decode_export_frame = |path: &Path| -> Result<Vec<u8>> {
            let result = crate::export_control::ExportControl::default().output(
                crate::export_control::hide_child_window(&mut Command::new(&tools.ffmpeg))
                    .args(["-hide_banner", "-loglevel", "error", "-ss", "0.5", "-i"])
                    .arg(path)
                    .args([
                        "-frames:v",
                        "1",
                        "-f",
                        "image2pipe",
                        "-vcodec",
                        "png",
                        "pipe:1",
                    ]),
            )?;
            ensure!(
                result.status.success(),
                "could not decode composition export frame: {}",
                String::from_utf8_lossy(&result.stderr)
            );
            Ok(result.stdout)
        };
        let proxy_export_frame = decode_export_frame(&proxy_export)?;
        let original_export_frame = decode_export_frame(&original_export)?;
        ensure!(
            image::load_from_memory(&proxy_export_frame)?.to_rgba8()
                == image::load_from_memory(&original_export_frame)?.to_rgba8(),
            "enabling proxy preview changed final export pixels"
        );
        fs::write(
            validation_dir.join("proxy-export-uses-original.png"),
            &proxy_export_frame,
        )?;

        let mut project = CompositionProject::new();
        project.width = 1920;
        project.height = 1080;
        project.fps = 30;
        project.duration = 3.0;
        let video_source = project.add_source(&video_a)?;
        let video_b_source = project.add_source(&video_b)?;
        let video_c_source = project.add_source(&video_c)?;
        let audio_source = project.add_source(&audio_a)?;
        let audio_b_source = project.add_source(&audio_b)?;
        let camera_source = project.add_source(&camera_av)?;
        let png_source = project.add_source(&still_png)?;
        let jpeg_source = project.add_source(&still_jpeg)?;
        let main_video = project.add_clip(&video_source, ClipKind::Video, 0.0)?;
        project.add_visual_effect(&main_video, VisualEffectKind::Brightness { amount: 0.04 })?;
        project.add_visual_effect(&main_video, VisualEffectKind::Contrast { amount: 1.15 })?;
        project.add_visual_effect(&main_video, VisualEffectKind::Saturation { amount: 1.2 })?;
        project.add_visual_effect(&main_video, VisualEffectKind::Hue { degrees: 8.0 })?;
        project.add_visual_effect(&main_video, VisualEffectKind::Sharpen { amount: 0.35 })?;
        let fitted_video_height = project
            .clips
            .iter()
            .find(|clip| clip.id == main_video)
            .unwrap()
            .scale_y
            * f64::from(
                project
                    .sources
                    .iter()
                    .find(|source| source.id == video_source)
                    .unwrap()
                    .height
                    .unwrap(),
            );
        {
            let main = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == main_video)
                .unwrap();
            // Exercise a real source in/out trim while preserving timeline
            // placement: discard 150 ms from the head and 300 ms from the tail.
            main.timeline_start = 0.15;
            main.source_start = 0.15;
            main.duration = 2.55;
            main.blur = 5.0;
            main.y = (f64::from(project.height) - fitted_video_height) / 2.0;
        }
        project
            .clips
            .iter_mut()
            .find(|clip| clip.id == main_video)
            .unwrap()
            .crop_left = 0.04;
        {
            let main = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == main_video)
                .unwrap();
            main.animated_properties.insert(
                AnimatedProperty::CropLeft,
                vec![
                    ScalarKeyframe {
                        time: 0.0,
                        value: 0.04,
                        easing_to_next: Easing::Linear,
                    },
                    ScalarKeyframe {
                        time: 1.5,
                        value: 0.12,
                        easing_to_next: Easing::Linear,
                    },
                ],
            );
            main.animated_properties.insert(
                AnimatedProperty::Blur,
                vec![
                    ScalarKeyframe {
                        time: 0.0,
                        value: 5.0,
                        easing_to_next: Easing::EaseInOut,
                    },
                    ScalarKeyframe {
                        time: 1.5,
                        value: 0.5,
                        easing_to_next: Easing::Linear,
                    },
                ],
            );
        }
        let overlay_track = project.add_track(TrackKind::Visual, "Overlay")?;
        let (camera_video, _camera_audio) = project.add_linked_av_clip(&camera_source, 0.0)?;
        {
            let camera = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == camera_video)
                .unwrap();
            camera.track_id = Some(overlay_track.clone());
            camera.scale_x = 0.3;
            camera.scale_y = 0.3;
            camera.x = 1_300.0;
            camera.y = 40.0;
        }
        for (source_id, at, x) in [(&png_source, 1.0, 1_300.0), (&jpeg_source, 2.0, 1_600.0)] {
            let image_clip =
                project.add_clip_to_track(source_id, ClipKind::Video, at, Some(&overlay_track))?;
            let clip = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == image_clip)
                .unwrap();
            clip.duration = 0.75;
            clip.scale_x = 0.4;
            clip.scale_y = 0.4;
            clip.x = x;
            clip.y = 700.0;
        }
        let solid_source = project.add_solid_color_source("F5C542")?;
        let solid_clip =
            project.add_clip_to_track(&solid_source, ClipKind::Video, 0.5, Some(&overlay_track))?;
        {
            let clip = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == solid_clip)
                .unwrap();
            clip.duration = 0.75;
            clip.scale_x = 0.2;
            clip.scale_y = 0.2;
            clip.x = 1_600.0;
            clip.y = 100.0;
        }
        let video_b_clip = project.add_clip_to_track(
            &video_b_source,
            ClipKind::Video,
            0.5,
            Some(&overlay_track),
        )?;
        project.add_visual_effect(
            &video_b_clip,
            VisualEffectKind::ChromaKey {
                key_color: "FF0000".into(),
                tolerance: 0.15,
                softness: 0.05,
            },
        )?;
        let _split_video = project.split_clip(&video_b_clip, 1.5)?;
        for clip in project
            .clips
            .iter_mut()
            .filter(|clip| clip.source_id.as_deref() == Some(video_b_source.as_str()))
        {
            clip.scale_x = 0.55;
            clip.scale_y = 0.55;
            clip.x = 130.0;
            clip.y = 300.0;
        }
        let video_c_clip = project.add_clip_to_track(
            &video_c_source,
            ClipKind::Video,
            0.0,
            Some(&overlay_track),
        )?;
        {
            let clip = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == video_c_clip)
                .unwrap();
            clip.scale_x = 0.5;
            clip.scale_y = 0.5;
            clip.x = 690.0;
            clip.y = 520.0;
        }
        let duplicate = project.duplicate_clip(&video_c_clip)?;
        let duplicate_clip = project
            .clips
            .iter_mut()
            .find(|clip| clip.id == duplicate)
            .unwrap();
        duplicate_clip.timeline_start = 0.0;
        duplicate_clip.x = 300.0;
        duplicate_clip.scale_x = 0.25;
        duplicate_clip.scale_y = 0.25;
        duplicate_clip.track_id = Some(overlay_track.clone());
        duplicate_clip.z_order = 3;
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::X,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 300.0,
                    easing_to_next: Easing::EaseInOut,
                },
                ScalarKeyframe {
                    time: 1.5,
                    value: 500.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::Y,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 200.0,
                    easing_to_next: Easing::Hold,
                },
                ScalarKeyframe {
                    time: 1.0,
                    value: 400.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::ScaleX,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 0.25,
                    easing_to_next: Easing::EaseIn,
                },
                ScalarKeyframe {
                    time: 1.5,
                    value: 0.35,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::ScaleY,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 0.25,
                    easing_to_next: Easing::EaseOut,
                },
                ScalarKeyframe {
                    time: 1.5,
                    value: 0.35,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::Rotation,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 0.0,
                    easing_to_next: Easing::Linear,
                },
                ScalarKeyframe {
                    time: 1.5,
                    value: 30.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        duplicate_clip.animated_properties.insert(
            AnimatedProperty::Opacity,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 0.45,
                    easing_to_next: Easing::EaseInOut,
                },
                ScalarKeyframe {
                    time: 1.5,
                    value: 1.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        let audio = project.add_clip(&audio_source, ClipKind::Audio, 0.0)?;
        {
            let audio_clip = project
                .clips
                .iter_mut()
                .find(|clip| clip.id == audio)
                .unwrap();
            audio_clip.gain = 0.65;
            audio_clip.fade_in = 0.25;
            audio_clip.fade_out = 0.25;
            audio_clip.animated_properties.insert(
                AnimatedProperty::Gain,
                vec![
                    ScalarKeyframe {
                        time: 0.0,
                        value: 0.0,
                        easing_to_next: Easing::EaseIn,
                    },
                    ScalarKeyframe {
                        time: 0.5,
                        value: 0.65,
                        easing_to_next: Easing::Linear,
                    },
                ],
            );
        }
        let audio_track = project.add_track(TrackKind::Audio, "Voiceover")?;
        let voiceover =
            project.add_clip_to_track(&audio_b_source, ClipKind::Audio, 0.4, Some(&audio_track))?;
        let voiceover_clip = project
            .clips
            .iter_mut()
            .find(|clip| clip.id == voiceover)
            .unwrap();
        voiceover_clip.gain = 0.5;
        voiceover_clip.fade_in = 0.15;
        voiceover_clip.fade_out = 0.25;
        voiceover_clip.animated_properties.insert(
            AnimatedProperty::Pan,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: -0.5,
                    easing_to_next: Easing::Linear,
                },
                ScalarKeyframe {
                    time: 2.0,
                    value: 0.5,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        let text = project.add_text_clip("Standalone composition", 0.0, 3.0)?;
        let text_clip = project
            .clips
            .iter_mut()
            .find(|clip| clip.id == text)
            .unwrap();
        text_clip.x = 80.0;
        text_clip.y = 850.0;
        text_clip.font_size = 48;
        text_clip.text_background_enabled = true;
        text_clip.text_background_color = "10243A".into();
        text_clip.text_background_opacity = 0.72;
        text_clip.text_background_padding = 16;
        text_clip.track_id = Some(overlay_track.clone());
        text_clip.animated_properties.insert(
            AnimatedProperty::FontSize,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 32.0,
                    easing_to_next: Easing::EaseOut,
                },
                ScalarKeyframe {
                    time: 1.0,
                    value: 48.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        text_clip.animated_properties.insert(
            AnimatedProperty::Opacity,
            vec![
                ScalarKeyframe {
                    time: 0.0,
                    value: 0.0,
                    easing_to_next: Easing::EaseOut,
                },
                ScalarKeyframe {
                    time: 0.6,
                    value: 1.0,
                    easing_to_next: Easing::Linear,
                },
            ],
        );
        project.validate()?;

        let project_path = temp.path().join("acceptance.sonocomp");
        project.save(&project_path)?;
        let reopened = CompositionProject::load(&project_path)?;
        ensure!(
            reopened.sources.len() == 9,
            "saved project lost imported media"
        );
        ensure!(
            reopened
                .sources
                .iter()
                .filter(|source| source.still_image)
                .count()
                == 2,
            "PNG/JPEG still-image media type was not retained after reopen"
        );
        let camera_links: Vec<_> = reopened
            .clips
            .iter()
            .filter(|clip| clip.source_id.as_deref() == Some(camera_source.as_str()))
            .collect();
        ensure!(
            camera_links.len() == 2
                && camera_links[0].link_group_id.is_some()
                && camera_links[0].link_group_id == camera_links[1].link_group_id,
            "camera video/audio tracks were not persisted as one linked pair"
        );
        let validation_dir = std::env::current_dir()?.join("artifacts/nle-validation");
        fs::create_dir_all(&validation_dir)?;
        let thumbnail = media_thumbnail_png(&video_a, 160, 90)?;
        let thumbnail_image = image::load_from_memory(&thumbnail)?;
        ensure!(
            thumbnail_image.width() <= 160
                && thumbnail_image.height() <= 90
                && thumbnail_image.width() > 0
                && thumbnail_image.height() > 0,
            "media thumbnail decode exceeded its bounded output dimensions"
        );
        fs::write(validation_dir.join("media-bin-thumbnail.png"), &thumbnail)?;
        let mut mask_fixture = CompositionProject::new();
        mask_fixture.width = 640;
        mask_fixture.height = 360;
        mask_fixture.fps = 30;
        mask_fixture.duration = 1.0;
        let green_source = mask_fixture.add_solid_color_source("30B060")?;
        let green_clip = mask_fixture.add_clip(&green_source, ClipKind::Video, 0.0)?;
        mask_fixture
            .add_visual_effect(&green_clip, VisualEffectKind::Brightness { amount: 0.03 })?;
        mask_fixture.clips[0].mask = Some(VisualMask {
            shape: VisualMaskShape::Rectangle,
            x: 0.25,
            y: 0.1,
            width: 0.5,
            height: 0.8,
            feather: 0.0,
            inverted: false,
        });
        mask_fixture.validate()?;
        let mask_rectangle =
            image::load_from_memory(&preview_png(&mask_fixture, 0.5, &Default::default())?)?
                .to_rgb8();
        ensure!(
            mask_rectangle.get_pixel(320, 180)[1] > 120 && mask_rectangle.get_pixel(20, 180)[1] < 8,
            "rectangle mask did not retain its interior and clear its exterior"
        );
        fs::write(
            validation_dir.join("mask-rectangle-hard.png"),
            preview_png(&mask_fixture, 0.5, &Default::default())?,
        )?;
        let mut mask_without_effect = mask_fixture.clone();
        mask_without_effect.clips[0].effects.clear();
        let mask_without_effect = image::load_from_memory(&preview_png(
            &mask_without_effect,
            0.5,
            &Default::default(),
        )?)?
        .to_rgb8();
        ensure!(
            mask_rectangle.get_pixel(320, 180)[1] != mask_without_effect.get_pixel(320, 180)[1]
                && mask_rectangle.get_pixel(20, 180)[1] < 8,
            "effect stack did not change retained mask pixels while preserving the masked exterior"
        );

        let mut feather_fixture = mask_fixture.clone();
        feather_fixture.clips[0].mask.as_mut().unwrap().feather = 0.08;
        let feather_frame =
            image::load_from_memory(&preview_png(&feather_fixture, 0.5, &Default::default())?)?
                .to_rgb8();
        ensure!(
            feather_frame.get_pixel(160, 180)[1] > 30
                && feather_frame.get_pixel(160, 180)[1] < feather_frame.get_pixel(320, 180)[1],
            "feather did not produce a partially transparent mask edge"
        );
        fs::write(
            validation_dir.join("mask-rectangle-feather.png"),
            preview_png(&feather_fixture, 0.5, &Default::default())?,
        )?;

        let mut ellipse_fixture = mask_fixture.clone();
        let ellipse = ellipse_fixture.clips[0].mask.as_mut().unwrap();
        ellipse.shape = VisualMaskShape::Ellipse;
        ellipse.y = 0.1;
        ellipse.height = 0.8;
        ellipse.inverted = true;
        let ellipse_frame =
            image::load_from_memory(&preview_png(&ellipse_fixture, 0.5, &Default::default())?)?
                .to_rgb8();
        ensure!(
            ellipse_frame.get_pixel(320, 180)[1] < 8 && ellipse_frame.get_pixel(165, 40)[1] > 100,
            "inverted ellipse mask did not swap the inside and outside"
        );
        fs::write(
            validation_dir.join("mask-ellipse-inverted.png"),
            preview_png(&ellipse_fixture, 0.5, &Default::default())?,
        )?;

        let mut animated_mask_fixture = mask_fixture.clone();
        {
            let clip = &mut animated_mask_fixture.clips[0];
            let mask = clip.mask.as_mut().unwrap();
            mask.x = 0.1;
            mask.width = 0.25;
            mask.y = 0.2;
            mask.height = 0.6;
            clip.add_keyframe(AnimatedProperty::MaskX, 0.0, 0.1, Easing::Linear)?;
            clip.add_keyframe(AnimatedProperty::MaskX, 0.8, 0.65, Easing::Linear)?;
        }
        let animated_early_bytes = preview_png(&animated_mask_fixture, 0.1, &Default::default())?;
        let animated_late_bytes = preview_png(&animated_mask_fixture, 0.9, &Default::default())?;
        let animated_early = image::load_from_memory(&animated_early_bytes)?.to_rgb8();
        let animated_late = image::load_from_memory(&animated_late_bytes)?.to_rgb8();
        ensure!(
            animated_early.get_pixel(128, 180)[1] > 100
                && animated_late.get_pixel(128, 180)[1] < 8
                && animated_early.get_pixel(480, 180)[1] < 8
                && animated_late.get_pixel(480, 180)[1] > 100,
            "animated mask did not move with the composition frame clock"
        );
        fs::write(
            validation_dir.join("mask-animated-before.png"),
            animated_early_bytes,
        )?;
        fs::write(
            validation_dir.join("mask-animated-after.png"),
            animated_late_bytes,
        )?;
        let mut transition_fixture = CompositionProject::new();
        transition_fixture.width = 640;
        transition_fixture.height = 360;
        transition_fixture.fps = 30;
        transition_fixture.duration = 3.0;
        let red_source = transition_fixture.add_solid_color_source("E02020")?;
        let blue_source = transition_fixture.add_solid_color_source("2040E0")?;
        let red_clip = transition_fixture.add_clip(&red_source, ClipKind::Video, 0.0)?;
        let blue_clip = transition_fixture.add_clip(&blue_source, ClipKind::Video, 1.0)?;
        transition_fixture
            .clips
            .iter_mut()
            .find(|clip| clip.id == red_clip)
            .unwrap()
            .duration = 2.0;
        let transition_id = transition_fixture.add_transition_between(
            &red_clip,
            &blue_clip,
            TransitionKind::CrossDissolve,
            1.0,
        )?;
        let linear_quarter = image::load_from_memory(&preview_png(
            &transition_fixture,
            1.25,
            &Default::default(),
        )?)?
        .to_rgb8()
        .get_pixel(320, 180)
        .0;
        transition_fixture.set_transition_easing(&transition_id, Easing::EaseInOut)?;
        let eased_quarter_bytes = preview_png(&transition_fixture, 1.25, &Default::default())?;
        let eased_quarter = image::load_from_memory(&eased_quarter_bytes)?
            .to_rgb8()
            .get_pixel(320, 180)
            .0;
        ensure!(
            eased_quarter != linear_quarter && eased_quarter[2] < linear_quarter[2],
            "ease-in-out transition progress did not differ from its linear counterpart"
        );
        fs::write(
            validation_dir.join("transition-ease-in-out-quarter.png"),
            eased_quarter_bytes,
        )?;
        transition_fixture.validate()?;
        let cross_bytes = preview_png(&transition_fixture, 1.5, &Default::default())?;
        let cross_frame = image::load_from_memory(&cross_bytes)?.to_rgb8();
        let cross_pixel = cross_frame.get_pixel(320, 180);
        ensure!(
            cross_pixel[0] > 35 && cross_pixel[2] > 80,
            "cross dissolve midpoint did not blend both source colors: {cross_pixel:?}"
        );
        fs::write(
            validation_dir.join("transition-cross-dissolve-midpoint.png"),
            cross_bytes,
        )?;
        transition_fixture.update_transition(&transition_id, TransitionKind::DipToBlack, 1.0)?;
        let dip_black_bytes = preview_png(&transition_fixture, 1.5, &Default::default())?;
        let dip_black = image::load_from_memory(&dip_black_bytes)?.to_rgb8();
        ensure!(
            dip_black
                .get_pixel(320, 180)
                .0
                .iter()
                .map(|value| u16::from(*value))
                .sum::<u16>()
                < 12,
            "dip-to-black midpoint was not black"
        );
        fs::write(
            validation_dir.join("transition-dip-to-black-midpoint.png"),
            dip_black_bytes,
        )?;
        transition_fixture.update_transition(&transition_id, TransitionKind::DipToWhite, 1.0)?;
        let dip_white_bytes = preview_png(&transition_fixture, 1.5, &Default::default())?;
        let dip_white = image::load_from_memory(&dip_white_bytes)?.to_rgb8();
        ensure!(
            dip_white
                .get_pixel(320, 180)
                .0
                .iter()
                .all(|value| *value > 240),
            "dip-to-white midpoint was not white"
        );
        fs::write(
            validation_dir.join("transition-dip-to-white-midpoint.png"),
            dip_white_bytes,
        )?;

        // Keep source trimming, constant retiming, a transition overlap, and
        // preview/export parity on one real-media path. These are separate
        // editor features in the UI, but users need them to compose cleanly.
        let mut trimmed_retime_transition = CompositionProject::new();
        trimmed_retime_transition.width = 640;
        trimmed_retime_transition.height = 360;
        trimmed_retime_transition.fps = 30;
        trimmed_retime_transition.duration = 3.0;
        trimmed_retime_transition.export_preset = CompositionExportPreset::Draft;
        let outgoing_source = trimmed_retime_transition.add_source(&video_b)?;
        let incoming_source = trimmed_retime_transition.add_source(&video_a)?;
        let outgoing_clip =
            trimmed_retime_transition.add_clip(&outgoing_source, ClipKind::Video, 0.0)?;
        let incoming_clip =
            trimmed_retime_transition.add_clip(&incoming_source, ClipKind::Video, 1.0)?;
        {
            let outgoing = trimmed_retime_transition
                .clips
                .iter_mut()
                .find(|clip| clip.id == outgoing_clip)
                .context("trimmed transition outgoing clip missing")?;
            outgoing.duration = 2.0;
            outgoing.source_start = 0.5;
            outgoing.playback_rate = 0.5;
            let incoming = trimmed_retime_transition
                .clips
                .iter_mut()
                .find(|clip| clip.id == incoming_clip)
                .context("trimmed transition incoming clip missing")?;
            incoming.duration = 2.0;
            incoming.source_start = 0.25;
            incoming.playback_rate = 0.75;
        }
        trimmed_retime_transition.add_transition_between(
            &outgoing_clip,
            &incoming_clip,
            TransitionKind::CrossDissolve,
            1.0,
        )?;
        trimmed_retime_transition.validate()?;
        let transition_preview_bytes =
            preview_png(&trimmed_retime_transition, 1.5, &Default::default())?;
        let transition_preview_pixel = image::load_from_memory(&transition_preview_bytes)?
            .to_rgb8()
            .get_pixel(320, 180)
            .to_owned();
        fs::write(
            validation_dir.join("transition-trimmed-retimed-preview.png"),
            &transition_preview_bytes,
        )?;
        let transition_export_dir = tempfile::tempdir()?;
        let transition_export = transition_export_dir.path().join("retimed-transition.mp4");
        render(
            &trimmed_retime_transition,
            &transition_export,
            &Default::default(),
        )?;
        let transition_export_frame = crate::export_control::ExportControl::default().output(
            crate::export_control::hide_child_window(&mut Command::new(&tools.ffmpeg))
                .args(["-hide_banner", "-loglevel", "error", "-ss", "1.5", "-i"])
                .arg(&transition_export)
                .args([
                    "-frames:v",
                    "1",
                    "-f",
                    "image2pipe",
                    "-vcodec",
                    "png",
                    "pipe:1",
                ]),
        )?;
        ensure!(
            transition_export_frame.status.success() && !transition_export_frame.stdout.is_empty(),
            "could not decode trimmed/retimed transition export frame: {}",
            String::from_utf8_lossy(&transition_export_frame.stderr)
        );
        let transition_export_pixel = image::load_from_memory(&transition_export_frame.stdout)?
            .to_rgb8()
            .get_pixel(320, 180)
            .to_owned();
        ensure!(
            transition_preview_pixel
                .0
                .iter()
                .zip(transition_export_pixel.0.iter())
                .all(|(preview, export)| preview.abs_diff(*export) < 24),
            "trimmed/retimed transition preview/export diverged: preview={transition_preview_pixel:?}, export={transition_export_pixel:?}"
        );

        let mut effect_order_fixture = CompositionProject::new();
        effect_order_fixture.width = 640;
        effect_order_fixture.height = 360;
        effect_order_fixture.fps = 30;
        effect_order_fixture.duration = 1.0;
        let effect_source = effect_order_fixture.add_solid_color_source("4080C0")?;
        let effect_clip = effect_order_fixture.add_clip(&effect_source, ClipKind::Video, 0.0)?;
        let source_frame = image::load_from_memory(&preview_png(
            &effect_order_fixture,
            0.5,
            &Default::default(),
        )?)?
        .to_rgb8();
        let brightness_id = effect_order_fixture
            .add_visual_effect(&effect_clip, VisualEffectKind::Brightness { amount: 0.2 })?;
        let contrast_id = effect_order_fixture
            .add_visual_effect(&effect_clip, VisualEffectKind::Contrast { amount: 2.0 })?;
        effect_order_fixture
            .add_visual_effect(&effect_clip, VisualEffectKind::Gamma { amount: 1.8 })?;
        let ordered_bytes = preview_png(&effect_order_fixture, 0.5, &Default::default())?;
        let ordered_frame = image::load_from_memory(&ordered_bytes)?.to_rgb8();
        fs::write(
            validation_dir.join("effect-stack-brightness-contrast-gamma.png"),
            &ordered_bytes,
        )?;
        effect_order_fixture.move_visual_effect(&effect_clip, &contrast_id, -1)?;
        let reordered_bytes = preview_png(&effect_order_fixture, 0.5, &Default::default())?;
        let reordered_frame = image::load_from_memory(&reordered_bytes)?.to_rgb8();
        ensure!(
            ordered_frame.get_pixel(320, 180) != reordered_frame.get_pixel(320, 180),
            "reordering noncommutative effects did not change rendered pixels"
        );
        fs::write(
            validation_dir.join("effect-stack-contrast-brightness-gamma.png"),
            &reordered_bytes,
        )?;
        effect_order_fixture.move_visual_effect(&effect_clip, &brightness_id, 1)?;
        for effect in &mut effect_order_fixture.clips[0].effects {
            effect.enabled = false;
        }
        let bypass_bytes = preview_png(&effect_order_fixture, 0.5, &Default::default())?;
        let bypass_frame = image::load_from_memory(&bypass_bytes)?.to_rgb8();
        ensure!(
            bypass_frame.get_pixel(320, 180) == source_frame.get_pixel(320, 180),
            "bypassing the full effect stack did not restore the source color"
        );
        fs::write(
            validation_dir.join("effect-stack-bypassed.png"),
            bypass_bytes,
        )?;

        let mut animated_effect_fixture = CompositionProject::new();
        animated_effect_fixture.width = 640;
        animated_effect_fixture.height = 360;
        animated_effect_fixture.fps = 30;
        animated_effect_fixture.duration = 1.0;
        let animated_source = animated_effect_fixture.add_solid_color_source("404040")?;
        let animated_clip =
            animated_effect_fixture.add_clip(&animated_source, ClipKind::Video, 0.0)?;
        let animated_brightness = animated_effect_fixture
            .add_visual_effect(&animated_clip, VisualEffectKind::Brightness { amount: 0.0 })?;
        animated_effect_fixture.set_effect_keyframe(
            &animated_clip,
            &animated_brightness,
            EffectParameter::Amount,
            0.0,
            -0.2,
            Easing::Linear,
        )?;
        animated_effect_fixture.set_effect_keyframe(
            &animated_clip,
            &animated_brightness,
            EffectParameter::Amount,
            0.9,
            0.4,
            Easing::Linear,
        )?;
        let animated_start_bytes = preview_png(&animated_effect_fixture, 0.1, &Default::default())?;
        let animated_end_bytes = preview_png(&animated_effect_fixture, 0.8, &Default::default())?;
        let animated_start = image::load_from_memory(&animated_start_bytes)?.to_rgb8();
        let animated_end = image::load_from_memory(&animated_end_bytes)?.to_rgb8();
        let animated_start_pixel = animated_start.get_pixel(320, 180);
        let animated_end_pixel = animated_end.get_pixel(320, 180);
        ensure!(
            animated_end_pixel[0] > animated_start_pixel[0].saturating_add(35),
            "frame-evaluated brightness did not follow its keyframes: start={animated_start_pixel:?}, end={animated_end_pixel:?}"
        );
        fs::write(
            validation_dir.join("effect-brightness-animated-start.png"),
            animated_start_bytes,
        )?;
        fs::write(
            validation_dir.join("effect-brightness-animated-end.png"),
            &animated_end_bytes,
        )?;
        let animated_export_dir = tempfile::tempdir()?;
        let animated_export_path = animated_export_dir.path().join("animated-effect.mp4");
        render(
            &animated_effect_fixture,
            &animated_export_path,
            &Default::default(),
        )?;
        let animated_export_frame = Command::new(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", "0.8", "-i"])
            .arg(&animated_export_path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ])
            .output()?;
        ensure!(
            animated_export_frame.status.success() && !animated_export_frame.stdout.is_empty(),
            "could not decode animated-effect MP4 frame: {}",
            String::from_utf8_lossy(&animated_export_frame.stderr)
        );
        let animated_export_pixel = image::load_from_memory(&animated_export_frame.stdout)?
            .to_rgb8()
            .get_pixel(320, 180)
            .to_owned();
        ensure!(
            animated_export_pixel[0].abs_diff(animated_end_pixel[0]) < 20,
            "animated effect preview and MP4 export diverged: preview={animated_end_pixel:?}, export={animated_export_pixel:?}"
        );

        let mut adjustment_fixture = CompositionProject::new();
        adjustment_fixture.width = 640;
        adjustment_fixture.height = 360;
        adjustment_fixture.fps = 30;
        adjustment_fixture.duration = 2.0;
        let adjustment_source = adjustment_fixture.add_solid_color_source("404040")?;
        let _under_clip = adjustment_fixture.add_clip(&adjustment_source, ClipKind::Video, 0.0)?;
        let adjustment_id = adjustment_fixture.add_adjustment_clip(0.5, 0.5)?;
        adjustment_fixture.add_visual_effect(
            &adjustment_id,
            VisualEffectKind::Brightness { amount: 0.35 },
        )?;
        let adjustment_before_bytes = preview_png(&adjustment_fixture, 0.25, &Default::default())?;
        let adjustment_active_bytes = preview_png(&adjustment_fixture, 0.75, &Default::default())?;
        let adjustment_after_bytes = preview_png(&adjustment_fixture, 1.25, &Default::default())?;
        let adjustment_before = image::load_from_memory(&adjustment_before_bytes)?.to_rgb8();
        let adjustment_active = image::load_from_memory(&adjustment_active_bytes)?.to_rgb8();
        let adjustment_after = image::load_from_memory(&adjustment_after_bytes)?.to_rgb8();
        let before_pixel = adjustment_before.get_pixel(320, 180);
        let active_pixel = adjustment_active.get_pixel(320, 180);
        let after_pixel = adjustment_after.get_pixel(320, 180);
        ensure!(
            active_pixel[0] > before_pixel[0].saturating_add(30)
                && before_pixel == after_pixel,
            "adjustment should brighten composited pixels only in its active interval: before={before_pixel:?}, active={active_pixel:?}, after={after_pixel:?}"
        );
        fs::write(
            validation_dir.join("adjustment-layer-inactive.png"),
            adjustment_before_bytes,
        )?;
        fs::write(
            validation_dir.join("adjustment-layer-active.png"),
            adjustment_active_bytes,
        )?;
        let mut bypassed_adjustment = adjustment_fixture.clone();
        bypassed_adjustment
            .clips
            .iter_mut()
            .find(|clip| clip.id == adjustment_id)
            .context("adjustment layer missing from bypass fixture")?
            .enabled = false;
        let bypass_frame = image::load_from_memory(&preview_png(
            &bypassed_adjustment,
            0.75,
            &Default::default(),
        )?)?
        .to_rgb8();
        ensure!(
            bypass_frame.get_pixel(320, 180) == before_pixel,
            "disabling an adjustment layer should restore the unaffected composite"
        );
        let adjustment_export_workspace = tempfile::tempdir()?;
        let adjustment_export_path = adjustment_export_workspace.path().join("adjustment.mp4");
        render(
            &adjustment_fixture,
            &adjustment_export_path,
            &Default::default(),
        )?;
        let exported_frame = Command::new(&tools.ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-ss", "0.75", "-i"])
            .arg(&adjustment_export_path)
            .args([
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-vcodec",
                "png",
                "pipe:1",
            ])
            .output()?;
        ensure!(
            exported_frame.status.success() && !exported_frame.stdout.is_empty(),
            "could not decode adjustment-layer export frame: {}",
            String::from_utf8_lossy(&exported_frame.stderr)
        );
        let exported_pixel = image::load_from_memory(&exported_frame.stdout)?
            .to_rgb8()
            .get_pixel(320, 180)
            .to_owned();
        ensure!(
            exported_pixel[0] > before_pixel[0].saturating_add(25)
                && exported_pixel[0].abs_diff(active_pixel[0]) < 20,
            "MP4 export should match active adjustment preview within lossy codec tolerance: preview={active_pixel:?}, export={exported_pixel:?}"
        );

        let mut time_mapping_fixture = CompositionProject::new();
        time_mapping_fixture.width = 640;
        time_mapping_fixture.height = 360;
        time_mapping_fixture.fps = 30;
        time_mapping_fixture.duration = 1.4;
        time_mapping_fixture.sources.push(
            reopened
                .sources
                .iter()
                .find(|source| source.id == video_source)
                .context("synthetic video source missing from reopened fixture")?
                .clone(),
        );
        let retimed_clip = time_mapping_fixture.add_clip(&video_source, ClipKind::Video, 0.0)?;
        time_mapping_fixture.set_clip_source_time(&retimed_clip, SourceTimeMode::Forward, 0.5)?;
        let slow_early = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            0.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        let slow_late = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            1.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        ensure!(
            slow_early
                .pixels()
                .zip(slow_late.pixels())
                .any(|(a, b)| a.0 != b.0),
            "half-speed source mapping did not advance through the source"
        );
        fs::write(
            validation_dir.join("source-time-half-speed.png"),
            preview_png(&time_mapping_fixture, 0.8, &Default::default())?,
        )?;

        time_mapping_fixture.set_clip_source_time(&retimed_clip, SourceTimeMode::Forward, 2.0)?;
        let fast_early = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            0.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        let fast_late = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            1.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        ensure!(
            fast_early
                .pixels()
                .zip(fast_late.pixels())
                .any(|(a, b)| a.0 != b.0),
            "2x source mapping did not advance through the source"
        );
        fs::write(
            validation_dir.join("source-time-double-speed.png"),
            preview_png(&time_mapping_fixture, 0.8, &Default::default())?,
        )?;

        time_mapping_fixture.set_clip_source_time(&retimed_clip, SourceTimeMode::Reverse, 1.0)?;
        let reverse_early = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            0.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        let reverse_late = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            1.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        ensure!(
            reverse_early
                .pixels()
                .zip(reverse_late.pixels())
                .any(|(a, b)| a.0 != b.0),
            "reverse source mapping did not change source frames"
        );
        fs::write(
            validation_dir.join("source-time-reverse.png"),
            preview_png(&time_mapping_fixture, 0.2, &Default::default())?,
        )?;
        let retimed_output = temp.path().join("source-time-reverse.mp4");
        render(
            &time_mapping_fixture,
            &retimed_output,
            &crate::export_control::ExportControl::default(),
        )?;
        let retimed_probe = probe_media(&retimed_output)?;
        ensure!(
            retimed_probe.has_video && (retimed_probe.duration - 1.4).abs() < 0.1,
            "retimed export did not preserve the requested timeline duration"
        );

        time_mapping_fixture.clips[0].source_start = 1.0;
        time_mapping_fixture.set_clip_source_time(&retimed_clip, SourceTimeMode::Freeze, 1.0)?;
        let freeze_early = image::load_from_memory(&preview_png(
            &time_mapping_fixture,
            0.2,
            &Default::default(),
        )?)?
        .to_rgb8();
        let freeze_late_bytes = preview_png(&time_mapping_fixture, 1.2, &Default::default())?;
        let freeze_late = image::load_from_memory(&freeze_late_bytes)?.to_rgb8();
        ensure!(
            freeze_early
                .pixels()
                .zip(freeze_late.pixels())
                .all(|(a, b)| a.0 == b.0),
            "freeze frame output changed over timeline time"
        );
        fs::write(
            validation_dir.join("source-time-freeze-frame.png"),
            freeze_late_bytes,
        )?;
        let preview_bytes = preview_png(
            &reopened,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        fs::write(
            validation_dir.join("standalone-landscape-1920x1080.png"),
            &preview_bytes,
        )?;
        let preview = image::load_from_memory(&preview_bytes)?;
        ensure!(
            preview.width() == 1920 && preview.height() == 1080,
            "preview dimensions do not match the landscape composition"
        );
        let mut without_text_box = reopened.clone();
        let text_box = without_text_box
            .clips
            .iter_mut()
            .find(|clip| clip.kind == ClipKind::Text)
            .context("text fixture missing from reopened composition")?;
        text_box.text_background_enabled = false;
        let without_text_box = image::load_from_memory(&preview_png(
            &without_text_box,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?)?
        .to_rgb8();
        let text_box_pixels_changed = preview
            .to_rgb8()
            .pixels()
            .zip(without_text_box.pixels())
            .filter(|(with_box, without_box)| with_box.0 != without_box.0)
            .count();
        ensure!(
            text_box_pixels_changed > 100,
            "text background box did not change rendered pixels"
        );
        fs::write(
            validation_dir.join("text-background-box.png"),
            &preview_bytes,
        )?;
        let mut title_preset_fixture = CompositionProject::new();
        title_preset_fixture.width = 640;
        title_preset_fixture.height = 360;
        title_preset_fixture.duration = 3.0;
        let title_background = title_preset_fixture.add_solid_color_source("284A7A")?;
        title_preset_fixture.add_clip(&title_background, ClipKind::Video, 0.0)?;
        title_preset_fixture.add_title_from_preset("builtin-lower-third", 0.0, 3.0)?;
        let lower_third = preview_png(
            &title_preset_fixture,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        ensure!(
            lower_third != preview_bytes,
            "lower-third title preset did not change the rendered frame"
        );
        fs::write(
            validation_dir.join("title-preset-lower-third.png"),
            &lower_third,
        )?;
        let mut generated_shapes = CompositionProject::new();
        generated_shapes.width = 640;
        generated_shapes.height = 360;
        generated_shapes.duration = 2.0;
        let shape_background = generated_shapes.add_solid_color_source("203050")?;
        generated_shapes.add_clip(&shape_background, ClipKind::Video, 0.0)?;
        let rectangle =
            generated_shapes.add_shape_source(GeneratedShapeKind::Rectangle, "20B0E0")?;
        generated_shapes.add_clip(&rectangle, ClipKind::Video, 0.0)?;
        let ellipse = generated_shapes.add_shape_source(GeneratedShapeKind::Ellipse, "F5C542")?;
        generated_shapes.add_clip(&ellipse, ClipKind::Video, 0.0)?;
        let line = generated_shapes.add_shape_source(GeneratedShapeKind::Line, "FFFFFF")?;
        generated_shapes.add_clip(&line, ClipKind::Video, 0.0)?;
        fs::write(
            validation_dir.join("generated-basic-shapes.png"),
            preview_png(&generated_shapes, 0.5, &Default::default())?,
        )?;
        let mut without_chroma = reopened.clone();
        for clip in without_chroma
            .clips
            .iter_mut()
            .filter(|clip| clip.source_id.as_deref() == Some(video_b_source.as_str()))
        {
            for effect in &mut clip.effects {
                effect.enabled = false;
            }
        }
        let no_chroma_preview = preview_png(
            &without_chroma,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        let no_chroma = image::load_from_memory(&no_chroma_preview)?.to_rgb8();
        let keyed_rgb = preview.to_rgb8();
        let changed_chroma_pixels = keyed_rgb
            .pixels()
            .zip(no_chroma.pixels())
            .filter(|(with_key, without_key)| with_key.0 != without_key.0)
            .count();
        ensure!(
            changed_chroma_pixels > 100,
            "chroma-key effect did not change the rendered source pixel"
        );
        let mut without_stack = reopened.clone();
        without_stack
            .clips
            .iter_mut()
            .find(|clip| clip.id == main_video)
            .context("main video clip missing from reopened composition")?
            .effects
            .clear();
        let without_stack_preview = preview_png(
            &without_stack,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        let with_stack_rgb = preview.to_rgb8();
        let without_stack_rgb = image::load_from_memory(&without_stack_preview)?.to_rgb8();
        ensure!(
            with_stack_rgb
                .pixels()
                .zip(without_stack_rgb.pixels())
                .any(|(with, without)| with.0 != without.0),
            "ordered effect stack did not change rendered pixels"
        );
        let still_preview = preview_png(
            &reopened,
            1.25,
            &crate::export_control::ExportControl::default(),
        )?;
        fs::write(
            validation_dir.join("standalone-png-jpeg-and-solid.png"),
            &still_preview,
        )?;
        let source_preview = preview_source_png(
            &reopened,
            &video_source,
            1.25,
            &crate::export_control::ExportControl::default(),
        )?;
        fs::write(
            validation_dir.join("source-monitor-video-at-1.25s.png"),
            &source_preview,
        )?;
        let source_image = image::load_from_memory(&source_preview)?;
        ensure!(
            source_image.width() == 1920 && source_image.height() == 1080,
            "source monitor dimensions do not match the composition preview"
        );

        let mut vertical = reopened.clone();
        vertical.width = 1080;
        vertical.height = 1920;
        vertical.export_preset = CompositionExportPreset::HighQuality;
        if let Some(main) = vertical.clips.iter_mut().find(|clip| clip.id == main_video) {
            main.x = 0.0;
            main.y = (1920.0 - 360.0 * 1.6875) / 2.0;
            main.scale_x = 1.6875;
            main.scale_y = 1.6875;
            main.crop_left = 0.0;
        }
        let vertical_background = vertical.duplicate_clip(&main_video)?;
        if let Some(background) = vertical
            .clips
            .iter_mut()
            .find(|clip| clip.id == vertical_background)
        {
            background.timeline_start = 0.0;
            background.source_start = 0.0;
            background.duration = vertical.duration;
            background.scale_x = 5.333_333;
            background.scale_y = 5.333_333;
            background.x = -1_166.667;
            background.y = 0.0;
            background.blur = 18.0;
            background.opacity = 0.75;
            background.z_order = -1;
        }
        let vertical_preview = preview_png(
            &vertical,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        fs::write(
            validation_dir.join("standalone-vertical-1080x1920.png"),
            &vertical_preview,
        )?;
        let vertical_preview = image::load_from_memory(&vertical_preview)?;
        ensure!(
            vertical_preview.width() == 1080 && vertical_preview.height() == 1920,
            "preview dimensions do not match the vertical composition"
        );
        let mut nonstandard = reopened.clone();
        nonstandard.width = 802;
        nonstandard.height = 450;
        let nonstandard_preview = preview_png(
            &nonstandard,
            0.75,
            &crate::export_control::ExportControl::default(),
        )?;
        let nonstandard_preview = image::load_from_memory(&nonstandard_preview)?;
        ensure!(
            nonstandard_preview.width() == 802 && nonstandard_preview.height() == 450,
            "preview dimensions do not match the nonstandard composition"
        );

        // Exercise the same production FFmpeg composition graph in the
        // continuous playback form. These pipes are bounded by their readers
        // and expose the frame/audio data consumed by the CPAL editor output.
        let playback_work = temp.path().join("playback-work");
        fs::create_dir_all(&playback_work)?;
        let (video_program, video_args, playback_width, playback_height) =
            playback_video_command(&reopened, &playback_work)?;
        let mut video_process = Command::new(video_program)
            .args(video_args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        let frame_len = playback_width as usize * playback_height as usize * 4;
        let mut frame = vec![0; frame_len];
        video_process
            .stdout
            .take()
            .context("video pipe stdout missing")?
            .read_exact(&mut frame)?;
        ensure!(
            frame.iter().any(|byte| *byte != 0),
            "continuous video preview returned an empty frame"
        );
        video_process.kill()?;
        let _ = video_process.wait()?;

        let audio_work = temp.path().join("playback-audio-work");
        fs::create_dir_all(&audio_work)?;
        let (audio_program, audio_args) = playback_audio_command(&reopened, 48_000, &audio_work)?;
        let audio_output = Command::new(audio_program).args(audio_args).output()?;
        ensure!(
            audio_output.status.success(),
            "continuous audio preview failed: {}",
            String::from_utf8_lossy(&audio_output.stderr)
        );
        ensure!(
            audio_output.stdout.iter().any(|byte| *byte != 0),
            "continuous audio preview returned silence for the sine fixture"
        );

        let output = temp.path().join("standalone-acceptance.mp4");
        render(
            &reopened,
            &output,
            &crate::export_control::ExportControl::default(),
        )?;
        ensure!(output.is_file(), "composition export did not create an MP4");
        let mut ranged = reopened.clone();
        ranged.set_export_range(0.5, 1.5)?;
        let ranged_output = temp.path().join("standalone-range-acceptance.mp4");
        render(
            &ranged,
            &ranged_output,
            &crate::export_control::ExportControl::default(),
        )?;
        validate_export(
            &tools.ffprobe,
            &ranged_output,
            &ranged,
            ranged.export_duration(),
        )?;
        let vertical_output = temp.path().join("standalone-vertical-acceptance.mp4");
        render(
            &vertical,
            &vertical_output,
            &crate::export_control::ExportControl::default(),
        )?;
        ensure!(
            vertical_output.is_file(),
            "vertical composition export did not create an MP4"
        );
        validate_export(
            &tools.ffprobe,
            &vertical_output,
            &vertical,
            vertical.export_duration(),
        )?;

        let nested_child_path = temp.path().join("nested-child.sonocomp");
        let mut nested_child = CompositionProject::new();
        nested_child.name = "Nested Child".into();
        nested_child.width = 640;
        nested_child.height = 360;
        nested_child.duration = 1.0;
        let nested_color = nested_child.add_solid_color_source("D02030")?;
        nested_child.add_clip(&nested_color, ClipKind::Video, 0.0)?;
        nested_child.save(&nested_child_path)?;

        let nested_middle_path = temp.path().join("nested-middle.sonocomp");
        let mut nested_middle = CompositionProject::new();
        nested_middle.name = "Nested Middle".into();
        nested_middle.width = 640;
        nested_middle.height = 360;
        nested_middle.duration = 1.0;
        let child_source = nested_middle.add_nested_composition_source(&nested_child_path)?;
        nested_middle.add_clip(&child_source, ClipKind::Video, 0.0)?;
        nested_middle.save(&nested_middle_path)?;

        let mut nested_parent = CompositionProject::new();
        nested_parent.name = "Nested Parent".into();
        nested_parent.width = 640;
        nested_parent.height = 360;
        nested_parent.duration = 1.0;
        let middle_source = nested_parent.add_nested_composition_source(&nested_middle_path)?;
        let parent_clip = nested_parent.add_clip(&middle_source, ClipKind::Video, 0.0)?;
        nested_parent
            .clips
            .iter_mut()
            .find(|clip| clip.id == parent_clip)
            .context("nested parent clip is missing")?
            .opacity = 0.9;
        let nested_preview = preview_png(
            &nested_parent,
            0.5,
            &crate::export_control::ExportControl::default(),
        )?;
        let nested_image = image::load_from_memory(&nested_preview)?.to_rgb8();
        let nested_pixel = nested_image.get_pixel(320, 180).0;
        ensure!(
            nested_pixel[0] > nested_pixel[1] && nested_pixel[0] > nested_pixel[2],
            "two-level nested composition did not render the child red source"
        );
        fs::write(
            validation_dir.join("nested-composition-two-level.png"),
            &nested_preview,
        )?;

        let mut edited_child = CompositionProject::load(&nested_child_path)?;
        edited_child.sources[0].solid_color = Some("20D050".into());
        edited_child.save(&nested_child_path)?;
        let updated_preview = preview_png(
            &nested_parent,
            0.5,
            &crate::export_control::ExportControl::default(),
        )?;
        let updated_pixel = image::load_from_memory(&updated_preview)?
            .to_rgb8()
            .get_pixel(320, 180)
            .0;
        ensure!(
            updated_pixel[1] > updated_pixel[0] && updated_pixel[1] > updated_pixel[2],
            "parent preview did not resolve a changed child composition"
        );
        fs::write(
            validation_dir.join("nested-composition-two-level-after-edit.png"),
            &updated_preview,
        )?;
        let nested_export = temp.path().join("nested-composition-export.mp4");
        render(
            &nested_parent,
            &nested_export,
            &crate::export_control::ExportControl::default(),
        )?;
        validate_export(
            &tools.ffprobe,
            &nested_export,
            &nested_parent,
            nested_parent.duration,
        )?;
        Ok(())
    }

    #[test]
    #[ignore = "requires app-discoverable FFmpeg/FFprobe and the checked-in local video fixtures"]
    fn real_ffmpeg_template_reuse_across_ordinary_sources_portrait_and_gameplay() -> Result<()> {
        use anyhow::ensure;

        let tools = crate::ffmpeg::inspect_existing()?
            .context("presentation-template acceptance requires the app-managed FFmpeg pair")?;
        let root = std::env::current_dir()?;
        let temp = tempfile::tempdir()?;
        let validation_dir = root.join("artifacts/presentation-template-validation");
        fs::create_dir_all(&validation_dir)?;

        // Keep the production test deterministic and short while retaining real
        // frames from two unrelated local MP4s.
        let ordinary_a = temp.path().join("ordinary-a.mp4");
        let ordinary_b = temp.path().join("ordinary-b.mp4");
        for (source, output, seek) in [
            (root.join("TestingSuite/Test1.mp4"), &ordinary_a, "12"),
            (root.join("TestingSuite/Test2.mp4"), &ordinary_b, "3"),
        ] {
            ensure!(
                source.is_file(),
                "ordinary-media fixture is missing: {}",
                source.display()
            );
            let result = crate::export_control::ExportControl::default().output(
                crate::export_control::hide_child_window(
                    Command::new(&tools.ffmpeg)
                        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", seek, "-i"])
                        .arg(&source)
                        .args([
                            "-t",
                            "2",
                            "-vf",
                            "scale=640:360:force_original_aspect_ratio=decrease,pad=640:360:(ow-iw)/2:(oh-ih)/2",
                            "-an",
                            "-c:v",
                            "libx264",
                            "-preset",
                            "veryfast",
                            "-crf",
                            "24",
                            "-pix_fmt",
                            "yuv420p",
                        ])
                        .arg(output),
                ),
            )?;
            ensure!(
                result.status.success(),
                "could not prepare local MP4 fixture: {}",
                String::from_utf8_lossy(&result.stderr)
            );
        }

        let add_source = |project: &mut CompositionProject, path: &Path| -> Result<String> {
            let path = path.canonicalize()?;
            let probe = probe_media_offline_with_tools(&path, &tools)?;
            let source = MediaSource {
                id: unique_id("template-fixture-source"),
                display_name: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "Video".into()),
                path,
                has_video: probe.has_video,
                has_audio: probe.has_audio,
                duration: if probe.still_image {
                    0.0
                } else {
                    probe.duration
                },
                width: probe.width,
                height: probe.height,
                still_image: probe.still_image,
                ..Default::default()
            };
            ensure!(
                source.has_video,
                "template fixture source has no video stream"
            );
            let id = source.id.clone();
            project.sources.push(source);
            Ok(id)
        };

        // Build a data template from ordinary NLE objects. It has a blurred
        // full-frame base, a masked/detail layer, ordered effects, a dissolve,
        // an animated caption, and a generated graphic. No Watch or engine is
        // present in this path.
        let mut authored = CompositionProject::new();
        authored.name = "Reusable creator layout".into();
        authored.width = 1920;
        authored.height = 1080;
        authored.fps = 12;
        authored.duration = 2.0;
        let template_media = add_source(&mut authored, &ordinary_a)?;
        let background = authored.add_clip(&template_media, ClipKind::Video, 0.0)?;
        authored
            .clips
            .iter_mut()
            .find(|clip| clip.id == background)
            .unwrap()
            .blur = 12.0;
        authored.add_visual_effect(&background, VisualEffectKind::Saturation { amount: 0.75 })?;

        let overlay_track = authored.add_track(TrackKind::Visual, "Inset and titles")?;
        let first = authored.add_clip_to_track(
            &template_media,
            ClipKind::Video,
            0.0,
            Some(&overlay_track),
        )?;
        let next = authored.add_clip_to_track(
            &template_media,
            ClipKind::Video,
            1.25,
            Some(&overlay_track),
        )?;
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == first)
                .unwrap();
            clip.duration = 1.5;
            clip.scale_x = 0.9;
            clip.scale_y = 0.9;
            clip.x = 672.0;
            clip.y = 280.0;
            clip.mask = Some(VisualMask {
                shape: VisualMaskShape::Ellipse,
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0,
                feather: 0.0,
                inverted: false,
            });
        }
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == next)
                .unwrap();
            clip.timeline_start = 1.25;
            clip.duration = 0.75;
            clip.source_start = 1.25;
            clip.scale_x = 0.6;
            clip.scale_y = 0.6;
            clip.x = 1080.0;
            clip.y = 420.0;
        }
        authored.add_visual_effect(&first, VisualEffectKind::Brightness { amount: 0.08 })?;
        authored.add_transition_between(&first, &next, TransitionKind::CrossDissolve, 0.2)?;

        let title = authored.add_text_clip("{{param:title|A reusable title}}", 0.0, 2.0)?;
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == title)
                .unwrap();
            clip.x = 960.0;
            clip.y = 890.0;
            clip.font_size = 72;
            clip.text_alignment = TextAlignment::Center;
            clip.outline_width = 6.0;
            clip.text_background_enabled = true;
            clip.text_background_color = "18243A".into();
            clip.text_background_opacity = 0.55;
            clip.text_background_padding = 18;
            clip.add_keyframe(AnimatedProperty::Opacity, 0.0, 0.0, Easing::EaseOut)?;
            clip.add_keyframe(AnimatedProperty::Opacity, 0.35, 1.0, Easing::Linear)?;
        }
        let graphic = authored.add_shape_source(GeneratedShapeKind::Ellipse, "27D8D0")?;
        let badge =
            authored.add_clip_to_track(&graphic, ClipKind::Video, 0.0, Some(&overlay_track))?;
        {
            let clip = authored
                .clips
                .iter_mut()
                .find(|clip| clip.id == badge)
                .unwrap();
            clip.scale_x = 0.2;
            clip.scale_y = 0.2;
            clip.x = 1500.0;
            clip.y = 120.0;
            clip.duration = 2.0;
        }
        authored.validate()?;
        let template = CompositionTemplate::from_project(&authored, "Reusable creator layout")?;
        ensure!(
            template.source_slots.len() == 1,
            "template should expose one reusable video slot"
        );
        ensure!(
            template.canvas_fill_clip_ids.contains(&background),
            "full-frame background was not recognized for responsive cover layout"
        );
        ensure!(
            template
                .text_parameters
                .iter()
                .any(|parameter| parameter.name == "title"),
            "template title parameter was not exposed"
        );
        let template_path = validation_dir.join("reusable-creator-layout.sonotemplate");
        template.save(&template_path)?;
        let template = CompositionTemplate::load(&template_path)?;

        let mut landscape = CompositionProject::new();
        landscape.name = "Ordinary landscape from template".into();
        landscape.width = 1920;
        landscape.height = 1080;
        landscape.fps = 12;
        landscape.duration = 2.0;
        let landscape_source = add_source(&mut landscape, &ordinary_a)?;
        template.apply_to_project(
            &mut landscape,
            &HashMap::from([(template.source_slots[0].id.clone(), landscape_source)]),
            &HashMap::from([("title".into(), "Minimal landscape edit".into())]),
            true,
        )?;
        // The template result remains directly editable. Add an extra ordinary
        // NLE layer, alter an existing keyframe, then save/reopen before export.
        let extra_text = landscape.add_text_clip("Extra editable callout", 0.5, 1.0)?;
        let animated_text = landscape
            .clips
            .iter_mut()
            .find(|clip| clip.kind == ClipKind::Text && clip.text == "Minimal landscape edit")
            .context("template title is missing")?;
        animated_text.add_keyframe(AnimatedProperty::FontSize, 0.5, 88.0, Easing::EaseInOut)?;
        landscape.validate()?;
        let landscape_project_path = validation_dir.join("ordinary-landscape.sonocomp");
        landscape.save(&landscape_project_path)?;
        let landscape = CompositionProject::load(&landscape_project_path)?;
        ensure!(
            landscape.clips.iter().any(|clip| clip.id == extra_text),
            "extra user clip did not survive save/reopen"
        );
        let landscape_preview = preview_png(&landscape, 0.8, &Default::default())?;
        fs::write(
            validation_dir.join("ordinary-landscape-preview.png"),
            &landscape_preview,
        )?;
        let landscape_transition_preview = preview_png(&landscape, 1.35, &Default::default())?;
        fs::write(
            validation_dir.join("ordinary-landscape-transition-midpoint.png"),
            &landscape_transition_preview,
        )?;
        let landscape_output = temp.path().join("ordinary-landscape-export.mp4");
        render(&landscape, &landscape_output, &Default::default())?;
        validate_export(
            &tools.ffprobe,
            &landscape_output,
            &landscape,
            landscape.duration,
        )?;
        fs::copy(
            &landscape_output,
            validation_dir.join("ordinary-landscape-export.mp4"),
        )?;

        let mut portrait = CompositionProject::new();
        portrait.name = "Second source portrait from same template".into();
        portrait.width = 1080;
        portrait.height = 1920;
        portrait.fps = 12;
        portrait.duration = 2.0;
        let second_source = add_source(&mut portrait, &ordinary_b)?;
        template.apply_to_project(
            &mut portrait,
            &HashMap::from([(template.source_slots[0].id.clone(), second_source)]),
            &HashMap::from([("title".into(), "Different source / vertical layout".into())]),
            true,
        )?;
        let portrait_background = portrait
            .clips
            .iter()
            .find(|clip| clip.blur > 0.0)
            .context("responsive template background is missing")?;
        let portrait_source = portrait
            .sources
            .iter()
            .find(|source| Some(source.id.as_str()) == portrait_background.source_id.as_deref())
            .context("responsive template background source is missing")?;
        ensure!(
            portrait_background.scale_y * f64::from(portrait_source.height.unwrap_or_default())
                >= f64::from(portrait.height) - 1.0,
            "responsive background does not cover the portrait canvas"
        );
        ensure!(
            portrait_background.x < 0.0,
            "portrait background did not center-crop horizontally"
        );
        portrait.validate()?;
        let portrait_preview = preview_png(&portrait, 0.8, &Default::default())?;
        fs::write(
            validation_dir.join("ordinary-portrait-preview.png"),
            &portrait_preview,
        )?;
        let portrait_output = temp.path().join("ordinary-portrait-export.mp4");
        render(&portrait, &portrait_output, &Default::default())?;
        validate_export(
            &tools.ffprobe,
            &portrait_output,
            &portrait,
            portrait.duration,
        )?;
        fs::copy(
            &portrait_output,
            validation_dir.join("ordinary-portrait-export.mp4"),
        )?;

        // A real Sono-Renderer Larp render is consumed as ordinary media by the
        // same source slot. No gameplay runtime is opened in this NLE test.
        let gameplay_video = root.join("TestingSuite/Next Sekai Engine/levels/larp 64x/render.mp4");
        ensure!(
            gameplay_video.is_file(),
            "pre-rendered Larp gameplay MP4 fixture is missing"
        );
        let mut gameplay = CompositionProject::new();
        gameplay.name = "Pre-rendered Sonolus gameplay in template".into();
        gameplay.width = 1080;
        gameplay.height = 1920;
        gameplay.fps = 12;
        gameplay.duration = 2.0;
        let gameplay_source = add_source(&mut gameplay, &gameplay_video)?;
        template.apply_to_project(
            &mut gameplay,
            &HashMap::from([(template.source_slots[0].id.clone(), gameplay_source)]),
            &HashMap::from([("title".into(), "Larp gameplay / same NLE objects".into())]),
            true,
        )?;
        ensure!(
            gameplay.sources.len()
                == template.source_slots.len() + template.generated_sources.len(),
            "gameplay template inserted sources outside its declared media slots and procedural assets"
        );
        let gameplay_value = serde_json::to_value(&gameplay)?;
        ensure!(
            gameplay_value.get("engine").is_none()
                && gameplay_value.get("level").is_none()
                && gameplay_value.get("watch").is_none(),
            "standalone NLE composition contains Sonolus runtime data"
        );
        let gameplay_preview = preview_png(&gameplay, 0.8, &Default::default())?;
        fs::write(
            validation_dir.join("larp-gameplay-portrait-preview.png"),
            &gameplay_preview,
        )?;
        let gameplay_output = temp.path().join("larp-gameplay-portrait-export.mp4");
        render(&gameplay, &gameplay_output, &Default::default())?;
        validate_export(
            &tools.ffprobe,
            &gameplay_output,
            &gameplay,
            gameplay.duration,
        )?;
        fs::copy(
            &gameplay_output,
            validation_dir.join("larp-gameplay-portrait-export.mp4"),
        )?;
        Ok(())
    }

    #[test]
    fn unrelated_engine_and_level_state_is_not_in_the_composition_schema() {
        let value = serde_json::to_value(CompositionProject::new()).unwrap();
        assert!(value.get("engine").is_none());
        assert!(value.get("level").is_none());
        assert!(value.get("watch").is_none());
        assert!(value.get("execution_mode").is_none());
    }
}
