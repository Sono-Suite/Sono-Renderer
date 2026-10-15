//! Deterministic, renderer-owned gameplay output watermarks.
//!
//! The fixed renderer attribution and the optional moving protection mark are
//! separate presentations controlled by one master switch. Movement is a pure
//! function of configuration, seed, output dimensions, and renderer time.

use crate::render_ui::{UiBitmap, UiBitmapFont};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const ATTRIBUTION_TEXT: &str = "\u{203b}Made with\u{300c}Sono-Renderer\u{300d}by Nexint.";
const WATERMARK_PROFILE_FORMAT: &str = "sono-renderer-watermark-profile";
const WATERMARK_PROFILE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WatermarkConfig {
    /// Master switch for attribution and moving protection marks.
    pub enabled: bool,
    pub attribution: AttributionConfig,
    pub moving: MovingWatermarkConfig,
    /// Bitmap image ids and font atlases/glyphs, resolved relative to the
    /// watermark profile when loaded from a profile file.
    pub assets: BTreeMap<String, PathBuf>,
    pub fonts: BTreeMap<String, UiBitmapFont>,
    #[serde(skip, default)]
    pub(crate) loaded_assets: BTreeMap<String, Arc<UiBitmap>>,
    #[serde(skip, default)]
    profile_directory: Option<PathBuf>,
}

impl Default for WatermarkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            attribution: AttributionConfig::default(),
            moving: MovingWatermarkConfig::default(),
            assets: BTreeMap::new(),
            fonts: BTreeMap::new(),
            loaded_assets: BTreeMap::new(),
            profile_directory: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AttributionConfig {
    /// Optional bitmap-font name from `fonts`; None uses the built-in font.
    pub font: Option<String>,
    pub font_size: f64,
    pub opacity: f64,
    pub margin: f64,
}

impl Default for AttributionConfig {
    fn default() -> Self {
        Self {
            font: None,
            font_size: 18.0,
            opacity: 0.72,
            margin: 10.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MovingWatermarkConfig {
    pub enabled: bool,
    pub content: WatermarkContent,
    pub font: Option<String>,
    pub font_size: f64,
    /// Target pixel height for image/sprite content before `scale`.
    pub image_height: f64,
    pub opacity: f64,
    pub scale: f64,
    pub rotation_degrees: f64,
    pub color: [u8; 4],
    /// Normalized frame coordinates for the selected anchor.
    pub start: [f64; 2],
    pub anchor: WatermarkAnchor,
    /// Normalized `[left, top, width, height]` safe movement rectangle.
    pub bounds: [f64; 4],
    pub movement: WatermarkMovement,
    pub timing: WatermarkTiming,
    pub animation: Option<WatermarkAnimation>,
    pub layer: WatermarkLayer,
    pub seed: u64,
}

impl Default for MovingWatermarkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            content: WatermarkContent::Text {
                text: "Sono-Renderer".into(),
            },
            font: None,
            font_size: 42.0,
            image_height: 128.0,
            opacity: 0.16,
            scale: 1.0,
            rotation_degrees: -18.0,
            color: [255, 255, 255, 255],
            start: [0.5, 0.5],
            anchor: WatermarkAnchor::Center,
            bounds: [0.05, 0.05, 0.90, 0.90],
            movement: WatermarkMovement::default(),
            timing: WatermarkTiming::default(),
            animation: None,
            layer: WatermarkLayer::AfterGameplayUi,
            seed: 0x534f_4e4f_5252_0007,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WatermarkContent {
    Text {
        text: String,
    },
    /// The source rectangle is normalized `[x, y, width, height]` and allows
    /// the same PNG to serve as a sprite sheet or a standalone texture.
    Image {
        asset: String,
        #[serde(default = "full_source")]
        source: [f64; 4],
    },
}

impl Default for WatermarkContent {
    fn default() -> Self {
        Self::Text {
            text: "Sono-Renderer".into(),
        }
    }
}

fn full_source() -> [f64; 4] {
    [0.0, 0.0, 1.0, 1.0]
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkAnchor {
    TopLeft,
    TopCenter,
    TopRight,
    CenterLeft,
    #[default]
    Center,
    CenterRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl WatermarkAnchor {
    pub(crate) fn fractions(self) -> [f64; 2] {
        match self {
            Self::TopLeft => [0.0, 0.0],
            Self::TopCenter => [0.5, 0.0],
            Self::TopRight => [1.0, 0.0],
            Self::CenterLeft => [0.0, 0.5],
            Self::Center => [0.5, 0.5],
            Self::CenterRight => [1.0, 0.5],
            Self::BottomLeft => [0.0, 1.0],
            Self::BottomCenter => [0.5, 1.0],
            Self::BottomRight => [1.0, 1.0],
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkInterpolation {
    #[default]
    Linear,
    SmoothStep,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LinearBoundary {
    #[default]
    Bounce,
    Wrap,
    Clamp,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum WatermarkMovement {
    Static,
    /// Velocity is normalized screen widths/heights per second.
    Linear {
        velocity: [f64; 2],
        #[serde(default)]
        boundary: LinearBoundary,
    },
    Waypoints {
        points: Vec<[f64; 2]>,
        #[serde(default = "default_segment_seconds")]
        segment_seconds: f64,
        #[serde(default)]
        interpolation: WatermarkInterpolation,
        #[serde(default = "default_true")]
        repeat: bool,
    },
    /// Absolute-time deterministic random waypoints derived from `seed` and
    /// segment index. Rendering frames out of order cannot alter the path.
    RandomWaypoints {
        #[serde(default = "default_segment_seconds")]
        segment_seconds: f64,
        #[serde(default)]
        interpolation: WatermarkInterpolation,
    },
}

impl Default for WatermarkMovement {
    fn default() -> Self {
        Self::Static
    }
}

fn default_segment_seconds() -> f64 {
    2.0
}
fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum WatermarkLayer {
    BeforeGameplayUi,
    #[default]
    AfterGameplayUi,
    Topmost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WatermarkTiming {
    pub start_time: f64,
    /// None means visible indefinitely. A finite duration can be repeated.
    pub duration: Option<f64>,
    pub repeat: bool,
}

impl Default for WatermarkTiming {
    fn default() -> Self {
        Self {
            start_time: 0.0,
            duration: None,
            repeat: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WatermarkAnimation {
    pub period_seconds: f64,
    pub scale_amplitude: f64,
    pub rotation_amplitude_degrees: f64,
    /// Additive opacity variation, clamped to `[0, 1]`.
    pub opacity_amplitude: f64,
    pub phase_radians: f64,
}

impl Default for WatermarkAnimation {
    fn default() -> Self {
        Self {
            period_seconds: 4.0,
            scale_amplitude: 0.0,
            rotation_amplitude_degrees: 0.0,
            opacity_amplitude: 0.0,
            phase_radians: 0.0,
        }
    }
}

impl WatermarkConfig {
    /// Convert profile-relative assets into explicit paths before embedding a
    /// watermark profile in the shared renderer configuration.
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

    /// Resolve assets in an embedded config relative to its containing file.
    pub(crate) fn resolve_config_resources(&mut self, base: &Path) {
        for path in self.assets.values_mut() {
            if !path.as_os_str().is_empty() && !path.is_absolute() {
                *path = base.join(&*path);
            }
        }
        self.profile_directory = None;
        self.loaded_assets.clear();
    }

    pub(crate) fn relativize_config_resources(&mut self, base: &Path) {
        for path in self.assets.values_mut() {
            crate::render::relativize_path(path, base);
        }
    }

    pub fn load_profile(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let data = std::fs::read(path)
            .with_context(|| format!("reading watermark profile {}", path.display()))?;
        let mut document: serde_json::Value = serde_json::from_slice(&data)
            .with_context(|| format!("parsing watermark profile {}", path.display()))?;
        crate::render::strip_optional_flat_schema(
            &mut document,
            WATERMARK_PROFILE_FORMAT,
            WATERMARK_PROFILE_SCHEMA_VERSION,
            "watermark profile",
        )?;
        let mut config: Self = serde_json::from_value(document)
            .with_context(|| format!("parsing watermark profile {}", path.display()))?;
        config.profile_directory = path
            .parent()
            .map(Path::to_path_buf)
            .map(|parent| parent.canonicalize().unwrap_or(parent));
        config.load_external_data()?;
        config.validate()?;
        Ok(config)
    }

    pub fn load_external_data(&mut self) -> Result<()> {
        self.loaded_assets.clear();
        for (id, path) in &self.assets {
            if id.trim().is_empty() {
                bail!("watermark asset id cannot be empty");
            }
            let resolved = if path.is_absolute() {
                path.clone()
            } else {
                self.profile_directory
                    .as_ref()
                    .map_or_else(|| path.clone(), |root| root.join(path))
            };
            let image = image::ImageReader::open(&resolved)
                .with_context(|| format!("opening watermark image {}", resolved.display()))?
                .with_guessed_format()
                .with_context(|| format!("identifying watermark image {}", resolved.display()))?
                .decode()
                .with_context(|| format!("decoding watermark image {}", resolved.display()))?
                .to_rgba8();
            let (width, height) = image.dimensions();
            if width == 0 || height == 0 || width.max(height) > 4096 {
                bail!("watermark image {id:?} has invalid dimensions {width}x{height}");
            }
            self.loaded_assets.insert(
                id.clone(),
                Arc::new(UiBitmap {
                    width,
                    height,
                    rgba: Arc::new(image.into_raw()),
                }),
            );
        }
        self.validate_fonts()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        validate_visual(self.attribution.font_size, self.attribution.opacity)?;
        if !self.attribution.margin.is_finite() || self.attribution.margin < 0.0 {
            bail!("watermark attribution margin must be finite and nonnegative");
        }
        if self
            .attribution
            .font
            .as_ref()
            .is_some_and(|name| !self.fonts.contains_key(name))
        {
            bail!("watermark attribution refers to an unknown font");
        }
        let moving = &self.moving;
        validate_visual(moving.font_size, moving.opacity)?;
        if !moving.image_height.is_finite() || !(0.1..=8192.0).contains(&moving.image_height) {
            bail!("moving watermark image height must be finite and in 0.1..=8192");
        }
        if !moving.scale.is_finite() || !(0.001..=1000.0).contains(&moving.scale) {
            bail!("moving watermark scale must be finite and in 0.001..=1000");
        }
        if !moving.rotation_degrees.is_finite() || moving.rotation_degrees.abs() > 1.0e6 {
            bail!("moving watermark rotation must be finite and bounded");
        }
        if moving.color[3] == 0 && moving.opacity > 0.0 {
            // Fully transparent source is legal, but called out through the
            // regular opacity handling rather than rejecting author intent.
        }
        if moving
            .start
            .iter()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
        {
            bail!("moving watermark start coordinates must be within 0..=1");
        }
        let [x, y, w, h] = moving.bounds;
        if [x, y, w, h].iter().any(|v| !v.is_finite())
            || x < 0.0
            || y < 0.0
            || w <= 0.0
            || h <= 0.0
            || x + w > 1.0 + f64::EPSILON
            || y + h > 1.0 + f64::EPSILON
        {
            bail!("moving watermark bounds must be an in-frame normalized rectangle");
        }
        if moving
            .font
            .as_ref()
            .is_some_and(|name| !self.fonts.contains_key(name))
        {
            bail!("moving watermark refers to an unknown font");
        }
        match &moving.content {
            WatermarkContent::Text { text } if text.is_empty() || text.chars().count() > 1024 => {
                bail!("moving watermark text must contain 1..=1024 characters");
            }
            WatermarkContent::Image { asset, source } => {
                if !self.assets.contains_key(asset) {
                    bail!("moving watermark refers to unknown image asset {asset:?}");
                }
                if source.iter().any(|v| !v.is_finite())
                    || source[0] < 0.0
                    || source[1] < 0.0
                    || source[2] <= 0.0
                    || source[3] <= 0.0
                    || source[0] + source[2] > 1.0 + f64::EPSILON
                    || source[1] + source[3] > 1.0 + f64::EPSILON
                {
                    bail!("moving watermark image source must be an in-image normalized rectangle");
                }
            }
            _ => {}
        }
        match &moving.movement {
            WatermarkMovement::Static => {}
            WatermarkMovement::Linear { velocity, .. } => {
                if velocity.iter().any(|v| !v.is_finite() || v.abs() > 10.0) {
                    bail!("moving watermark velocity must be finite and bounded");
                }
            }
            WatermarkMovement::Waypoints {
                points,
                segment_seconds,
                ..
            } => {
                validate_waypoints(points, *segment_seconds)?;
            }
            WatermarkMovement::RandomWaypoints {
                segment_seconds, ..
            } => {
                validate_waypoints(&[], *segment_seconds)?;
            }
        }
        if !moving.timing.start_time.is_finite()
            || moving.timing.start_time < 0.0
            || moving
                .timing
                .duration
                .is_some_and(|v| !v.is_finite() || v <= 0.0)
        {
            bail!("moving watermark timing is invalid");
        }
        if let Some(animation) = &moving.animation {
            if !animation.period_seconds.is_finite()
                || animation.period_seconds <= 0.0
                || !animation.scale_amplitude.is_finite()
                || animation.scale_amplitude.abs() > 10.0
                || !animation.rotation_amplitude_degrees.is_finite()
                || animation.rotation_amplitude_degrees.abs() > 36000.0
                || !animation.opacity_amplitude.is_finite()
                || animation.opacity_amplitude.abs() > 1.0
                || !animation.phase_radians.is_finite()
            {
                bail!("moving watermark animation is invalid");
            }
        }
        self.validate_fonts()?;
        Ok(())
    }

    fn validate_fonts(&self) -> Result<()> {
        for (name, font) in &self.fonts {
            if name.trim().is_empty()
                || font.glyph_width == 0
                || font.glyph_height == 0
                || font.columns == 0
                || !font.advance.is_finite()
                || font.advance <= 0.0
                || (font.asset.is_empty() && font.glyph_assets.is_empty())
            {
                bail!("watermark font {name:?} has invalid bitmap-font settings");
            }
            if !font.asset.is_empty() {
                let Some(bitmap) = self.loaded_assets.get(&font.asset) else {
                    if self.assets.contains_key(&font.asset) {
                        continue;
                    }
                    bail!(
                        "watermark font {name:?} refers to missing atlas {:?}",
                        font.asset
                    );
                };
                if font.glyph_width > bitmap.width
                    || font.glyph_height > bitmap.height
                    || font.columns > bitmap.width / font.glyph_width
                {
                    bail!("watermark font {name:?} does not fit its image atlas");
                }
            }
            for asset in font.glyph_assets.values() {
                if !self.assets.contains_key(asset) {
                    bail!("watermark font {name:?} refers to missing glyph asset {asset:?}");
                }
            }
        }
        Ok(())
    }

    pub(crate) fn effective_moving_enabled(&self) -> bool {
        self.enabled && self.moving.enabled
    }
}

/// Draw a configured moving mark beneath the custom gameplay UI layer.
pub(crate) fn render_before_gameplay_ui(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &WatermarkConfig,
    time: f64,
) -> Result<()> {
    if config.effective_moving_enabled() && config.moving.layer == WatermarkLayer::BeforeGameplayUi
    {
        render_moving(rgb, width, height, config, time)?;
    }
    Ok(())
}

/// Draw gameplay UI and attribution in the normal order, followed by marks
/// explicitly assigned to the topmost layer.
pub(crate) fn render_after_gameplay_ui(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &WatermarkConfig,
    time: f64,
) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }
    render_attribution(rgb, width, height, config)?;
    if config.effective_moving_enabled() && config.moving.layer == WatermarkLayer::AfterGameplayUi {
        render_moving(rgb, width, height, config, time)?;
    }
    if config.effective_moving_enabled() && config.moving.layer == WatermarkLayer::Topmost {
        render_moving(rgb, width, height, config, time)?;
    }
    Ok(())
}

fn render_attribution(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &WatermarkConfig,
) -> Result<()> {
    if !config.enabled {
        return Ok(());
    }
    let [width_f, height_f] = [f64::from(width), f64::from(height)];
    let margin = config
        .attribution
        .margin
        .min(width_f * 0.25)
        .min(height_f * 0.25);
    let available_width = (width_f - margin * 2.0).max(0.1);
    let available_height = (height_f - margin * 2.0).max(0.1);
    let measured = crate::render_ui::measure_watermark_text(
        ATTRIBUTION_TEXT,
        config.attribution.font_size,
        config.attribution.font.as_deref(),
        &config.fonts,
        &config.loaded_assets,
    );
    let scale = 1.0_f64
        .min(available_width / measured[0].max(0.1))
        .min(available_height / measured[1].max(0.1));
    let center = [
        margin + measured[0] * scale * 0.5,
        height_f - margin - measured[1] * scale * 0.5,
    ];
    crate::render_ui::render_watermark_text(
        rgb,
        width,
        height,
        ATTRIBUTION_TEXT,
        [255; 4],
        config.attribution.font_size,
        config.attribution.font.as_deref(),
        &config.fonts,
        &config.loaded_assets,
        measured,
        center,
        scale,
        0.0,
        config.attribution.opacity,
    );
    Ok(())
}

fn render_moving(
    rgb: &mut [u8],
    width: u32,
    height: u32,
    config: &WatermarkConfig,
    time: f64,
) -> Result<()> {
    let moving = &config.moving;
    let (size, image) = match &moving.content {
        WatermarkContent::Text { text } => (
            crate::render_ui::measure_watermark_text(
                text,
                moving.font_size,
                moving.font.as_deref(),
                &config.fonts,
                &config.loaded_assets,
            ),
            None,
        ),
        WatermarkContent::Image { asset, source } => {
            let bitmap = config
                .loaded_assets
                .get(asset)
                .with_context(|| format!("watermark image asset {asset:?} was not loaded"))?;
            let source_px = normalized_source(*source, bitmap.width, bitmap.height);
            let aspect = f64::from(source_px[2]) / f64::from(source_px[3].max(1));
            (
                [moving.image_height * aspect, moving.image_height],
                Some((bitmap, source_px)),
            )
        }
    };
    let Some(pose) = pose_at(config, time, width, height, size) else {
        return Ok(());
    };
    let center = [
        pose.anchor_x * f64::from(width),
        pose.anchor_y * f64::from(height),
    ];
    match &moving.content {
        WatermarkContent::Text { text } => crate::render_ui::render_watermark_text(
            rgb,
            width,
            height,
            text,
            moving.color,
            moving.font_size,
            moving.font.as_deref(),
            &config.fonts,
            &config.loaded_assets,
            size,
            center,
            pose.scale,
            pose.rotation_degrees,
            pose.opacity,
        ),
        WatermarkContent::Image { .. } => {
            let (bitmap, source) = image.expect("image dimensions came from image content");
            crate::render_ui::render_watermark_image(
                rgb,
                width,
                height,
                bitmap,
                source,
                moving.color,
                center,
                [size[0] * pose.scale, size[1] * pose.scale],
                pose.rotation_degrees,
                pose.opacity,
            );
        }
    }
    Ok(())
}

fn normalized_source(source: [f64; 4], width: u32, height: u32) -> [u32; 4] {
    let x = (source[0] * f64::from(width)).round() as u32;
    let y = (source[1] * f64::from(height)).round() as u32;
    let right = ((source[0] + source[2]) * f64::from(width)).round() as u32;
    let bottom = ((source[1] + source[3]) * f64::from(height)).round() as u32;
    [
        x,
        y,
        right.saturating_sub(x).max(1),
        bottom.saturating_sub(y).max(1),
    ]
}

fn validate_waypoints(points: &[[f64; 2]], segment_seconds: f64) -> Result<()> {
    if !segment_seconds.is_finite() || segment_seconds <= 0.0 || segment_seconds > 1.0e6 {
        bail!("watermark movement segment duration must be finite and positive");
    }
    if points.len() > 4096
        || points
            .iter()
            .flatten()
            .any(|v| !v.is_finite() || !(0.0..=1.0).contains(v))
    {
        bail!("watermark waypoints must be at most 4096 normalized coordinates");
    }
    Ok(())
}

fn validate_visual(size: f64, opacity: f64) -> Result<()> {
    if !size.is_finite() || !(0.1..=8192.0).contains(&size) {
        bail!("watermark font size must be finite and in 0.1..=8192");
    }
    if !opacity.is_finite() || !(0.0..=1.0).contains(&opacity) {
        bail!("watermark opacity must be within 0..=1");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WatermarkPose {
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub scale: f64,
    pub rotation_degrees: f64,
    pub opacity: f64,
}

/// Return the deterministic moving-watermark pose that the renderer uses at
/// an absolute frame time. This is a logical placement observation for
/// diagnostics; it does not draw or mutate the watermark configuration.
pub fn sample_pose(
    config: &WatermarkConfig,
    time: f64,
    width: u32,
    height: u32,
) -> Result<Option<WatermarkPose>> {
    if !config.effective_moving_enabled() {
        return Ok(None);
    }
    let moving = &config.moving;
    let size = match &moving.content {
        WatermarkContent::Text { text } => crate::render_ui::measure_watermark_text(
            text,
            moving.font_size,
            moving.font.as_deref(),
            &config.fonts,
            &config.loaded_assets,
        ),
        WatermarkContent::Image { asset, source } => {
            let bitmap = config
                .loaded_assets
                .get(asset)
                .with_context(|| format!("watermark image asset {asset:?} was not loaded"))?;
            let source_px = normalized_source(*source, bitmap.width, bitmap.height);
            let aspect = f64::from(source_px[2]) / f64::from(source_px[3].max(1));
            [moving.image_height * aspect, moving.image_height]
        }
    };
    Ok(pose_at(config, time, width, height, size))
}

/// Return None when the optional mark is disabled/outside its timing window.
fn pose_at(
    config: &WatermarkConfig,
    time: f64,
    width: u32,
    height: u32,
    unscaled_size: [f64; 2],
) -> Option<WatermarkPose> {
    let moving = &config.moving;
    if !config.effective_moving_enabled() || !time.is_finite() {
        return None;
    }
    let local = time - moving.timing.start_time;
    if local < 0.0 {
        return None;
    }
    let local = if let Some(duration) = moving.timing.duration {
        if moving.timing.repeat {
            local.rem_euclid(duration)
        } else if local > duration {
            return None;
        } else {
            local
        }
    } else {
        local
    };

    let mut requested = movement_position(moving, local);
    let animation_phase = moving.animation.as_ref().map_or(0.0, |a| {
        (std::f64::consts::TAU * local / a.period_seconds + a.phase_radians).sin()
    });
    let animation = moving.animation.as_ref();
    let mut scale =
        moving.scale * (1.0 + animation.map_or(0.0, |a| a.scale_amplitude * animation_phase));
    let rotation = moving.rotation_degrees
        + animation.map_or(0.0, |a| a.rotation_amplitude_degrees * animation_phase);
    let opacity = (moving.opacity
        + animation.map_or(0.0, |a| a.opacity_amplitude * animation_phase))
    .clamp(0.0, 1.0);
    if !scale.is_finite() || scale <= 0.0 || !rotation.is_finite() {
        return None;
    }

    let width = f64::from(width.max(1));
    let height = f64::from(height.max(1));
    let mut box_width = unscaled_size[0].max(1.0) * scale;
    let mut box_height = unscaled_size[1].max(1.0) * scale;
    let angle = rotation.to_radians();
    let [left, top, bound_w, bound_h] = moving.bounds;
    let bx0 = left * width;
    let by0 = top * height;
    let bx1 = (left + bound_w) * width;
    let by1 = (top + bound_h) * height;
    let rotated_dimensions = |w: f64, h: f64| {
        [
            angle.cos().abs() * w + angle.sin().abs() * h,
            angle.sin().abs() * w + angle.cos().abs() * h,
        ]
    };
    let initial_extents = rotated_dimensions(box_width, box_height);
    let fit_scale = ((bx1 - bx0) / initial_extents[0].max(1.0))
        .min((by1 - by0) / initial_extents[1].max(1.0))
        .min(1.0);
    scale *= fit_scale;
    box_width *= fit_scale;
    box_height *= fit_scale;
    let [rotated_width, rotated_height] = rotated_dimensions(box_width, box_height);
    let half_w = rotated_width * 0.5;
    let half_h = rotated_height * 0.5;
    let anchor = moving.anchor.fractions();
    let center_x = requested[0] * width + (0.5 - anchor[0]) * box_width;
    let center_y = requested[1] * height + (0.5 - anchor[1]) * box_height;
    let clamped_center_x = center_x.clamp(bx0 + half_w, bx1 - half_w);
    let clamped_center_y = center_y.clamp(by0 + half_h, by1 - half_h);
    requested = [clamped_center_x / width, clamped_center_y / height];

    Some(WatermarkPose {
        anchor_x: requested[0],
        anchor_y: requested[1],
        scale,
        rotation_degrees: rotation,
        opacity,
    })
}

fn movement_position(moving: &MovingWatermarkConfig, elapsed: f64) -> [f64; 2] {
    match &moving.movement {
        WatermarkMovement::Static => moving.start,
        WatermarkMovement::Linear { velocity, boundary } => {
            let [left, top, width, height] = moving.bounds;
            [
                linear_coordinate(
                    moving.start[0],
                    velocity[0],
                    elapsed,
                    left,
                    left + width,
                    *boundary,
                ),
                linear_coordinate(
                    moving.start[1],
                    velocity[1],
                    elapsed,
                    top,
                    top + height,
                    *boundary,
                ),
            ]
        }
        WatermarkMovement::Waypoints {
            points,
            segment_seconds,
            interpolation,
            repeat,
        } => {
            let mut nodes = Vec::with_capacity(points.len() + 1);
            nodes.push(moving.start);
            nodes.extend(points.iter().copied());
            follow_waypoints(&nodes, elapsed, *segment_seconds, *interpolation, *repeat)
        }
        WatermarkMovement::RandomWaypoints {
            segment_seconds,
            interpolation,
        } => {
            let segment = (elapsed / segment_seconds).floor().max(0.0) as u64;
            let phase = (elapsed / segment_seconds - segment as f64).clamp(0.0, 1.0);
            let a = if segment == 0 {
                moving.start
            } else {
                random_point(moving.seed, segment, moving.bounds)
            };
            let b = random_point(moving.seed, segment.saturating_add(1), moving.bounds);
            lerp_point(a, b, interpolate(phase, *interpolation))
        }
    }
}

fn follow_waypoints(
    points: &[[f64; 2]],
    elapsed: f64,
    segment_seconds: f64,
    interpolation: WatermarkInterpolation,
    repeat: bool,
) -> [f64; 2] {
    if points.len() <= 1 {
        return points.first().copied().unwrap_or([0.5, 0.5]);
    }
    let segments = points.len() - 1;
    let mut step = (elapsed / segment_seconds).max(0.0);
    if repeat {
        step = step.rem_euclid(segments as f64);
    } else {
        step = step.min(segments as f64);
    }
    let index = (step.floor() as usize).min(segments - 1);
    let phase = if !repeat && step >= segments as f64 {
        1.0
    } else {
        step.fract()
    };
    lerp_point(
        points[index],
        points[index + 1],
        interpolate(phase, interpolation),
    )
}

fn interpolate(value: f64, interpolation: WatermarkInterpolation) -> f64 {
    match interpolation {
        WatermarkInterpolation::Linear => value,
        WatermarkInterpolation::SmoothStep => value * value * (3.0 - 2.0 * value),
    }
}

fn lerp_point(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn linear_coordinate(
    start: f64,
    velocity: f64,
    time: f64,
    minimum: f64,
    maximum: f64,
    boundary: LinearBoundary,
) -> f64 {
    let span = maximum - minimum;
    if span <= f64::EPSILON {
        return (minimum + maximum) * 0.5;
    }
    let offset = start.clamp(minimum, maximum) - minimum + velocity * time;
    match boundary {
        LinearBoundary::Clamp => minimum + offset.clamp(0.0, span),
        LinearBoundary::Wrap => minimum + offset.rem_euclid(span),
        LinearBoundary::Bounce => {
            let phase = offset.rem_euclid(span * 2.0);
            minimum
                + if phase <= span {
                    phase
                } else {
                    span * 2.0 - phase
                }
        }
    }
}

fn random_point(seed: u64, index: u64, bounds: [f64; 4]) -> [f64; 2] {
    let first = mix64(seed ^ index.wrapping_mul(0x9e37_79b9_7f4a_7c15));
    let second = mix64(first ^ 0xd1b5_4a32_d192_ed03);
    [
        bounds[0] + unit_f64(first) * bounds[2],
        bounds[1] + unit_f64(second) * bounds[3],
    ]
}

fn mix64(mut value: u64) -> u64 {
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn unit_f64(value: u64) -> f64 {
    (value >> 11) as f64 / ((1u64 << 53) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_profile_rejects_an_unsupported_future_schema() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future-watermark.json");
        std::fs::write(
            &path,
            br#"{"format":"sono-renderer-watermark-profile","schema_version":2}"#,
        )
        .unwrap();
        assert!(WatermarkConfig::load_profile(path)
            .unwrap_err()
            .to_string()
            .contains("newer than supported"));
    }

    #[test]
    fn master_switch_subordinates_moving_watermark() {
        let mut config = WatermarkConfig::default();
        config.moving.enabled = true;
        assert!(config.effective_moving_enabled());
        config.enabled = false;
        assert!(!config.effective_moving_enabled());
        assert!(pose_at(&config, 1.0, 1280, 720, [240.0, 42.0]).is_none());
    }

    #[test]
    fn random_waypoint_is_seekable_seeded_and_worker_independent() {
        let mut config = WatermarkConfig::default();
        config.moving.enabled = true;
        config.moving.movement = WatermarkMovement::RandomWaypoints {
            segment_seconds: 1.25,
            interpolation: WatermarkInterpolation::SmoothStep,
        };
        let a = pose_at(&config, 8.375, 1280, 720, [300.0, 48.0]).unwrap();
        let _ = pose_at(&config, 2.0, 1280, 720, [300.0, 48.0]).unwrap();
        let b = pose_at(&config, 8.375, 1280, 720, [300.0, 48.0]).unwrap();
        assert_eq!(a, b);
        config.moving.seed += 1;
        let changed = pose_at(&config, 8.375, 1280, 720, [300.0, 48.0]).unwrap();
        assert_ne!(
            [a.anchor_x, a.anchor_y],
            [changed.anchor_x, changed.anchor_y]
        );
    }

    #[test]
    fn linear_boundaries_and_waypoint_interpolation_are_configurable() {
        assert_eq!(
            linear_coordinate(0.5, 0.75, 1.0, 0.0, 1.0, LinearBoundary::Bounce),
            0.75
        );
        assert_eq!(
            linear_coordinate(0.5, 0.75, 1.0, 0.0, 1.0, LinearBoundary::Wrap),
            0.25
        );
        assert_eq!(
            linear_coordinate(0.5, 0.75, 1.0, 0.0, 1.0, LinearBoundary::Clamp),
            1.0
        );
        let nodes = [[0.2, 0.2], [0.8, 0.6]];
        assert_eq!(
            follow_waypoints(&nodes, 0.5, 1.0, WatermarkInterpolation::Linear, false),
            [0.5, 0.4]
        );
        let smooth = follow_waypoints(&nodes, 0.25, 1.0, WatermarkInterpolation::SmoothStep, false);
        assert!((smooth[0] - 0.29375).abs() < f64::EPSILON * 4.0);
        assert!((smooth[1] - 0.2625).abs() < f64::EPSILON * 4.0);
        assert_eq!(
            follow_waypoints(&nodes, 1.5, 1.0, WatermarkInterpolation::Linear, true),
            [0.5, 0.4]
        );
    }

    #[test]
    fn image_geometry_respects_bounds_on_portrait_and_landscape_frames() {
        let mut config = WatermarkConfig::default();
        config.moving.enabled = true;
        config.moving.start = [1.0, 1.0];
        config.moving.rotation_degrees = 31.0;
        for (width, height) in [(1280, 720), (768, 1024), (160, 90)] {
            let pose = pose_at(&config, 0.0, width, height, [420.0, 80.0]).unwrap();
            let angle = pose.rotation_degrees.to_radians();
            let w = 420.0 * pose.scale;
            let h = 80.0 * pose.scale;
            let extent_x = angle.cos().abs() * w + angle.sin().abs() * h;
            let extent_y = angle.sin().abs() * w + angle.cos().abs() * h;
            let left = pose.anchor_x * f64::from(width) - extent_x * 0.5;
            let top = pose.anchor_y * f64::from(height) - extent_y * 0.5;
            assert!(left >= 0.05 * f64::from(width) - 1.0);
            assert!(top >= 0.05 * f64::from(height) - 1.0);
            assert!(left + extent_x <= 0.95 * f64::from(width) + 1.0);
            assert!(top + extent_y <= 0.95 * f64::from(height) + 1.0);
        }
    }

    #[test]
    fn timing_and_animation_are_renderer_time_driven() {
        let mut config = WatermarkConfig::default();
        config.moving.enabled = true;
        config.moving.timing = WatermarkTiming {
            start_time: 2.0,
            duration: Some(3.0),
            repeat: false,
        };
        assert!(pose_at(&config, 1.99, 1280, 720, [200.0, 40.0]).is_none());
        assert!(pose_at(&config, 5.01, 1280, 720, [200.0, 40.0]).is_none());
        assert!(pose_at(&config, 2.0, 1280, 720, [200.0, 40.0]).is_some());
    }

    #[test]
    fn validation_rejects_invalid_asset_and_bounds_references() {
        let mut config = WatermarkConfig::default();
        config.moving.content = WatermarkContent::Image {
            asset: "missing".into(),
            source: full_source(),
        };
        assert!(config.validate().is_err());
        config.moving.content = WatermarkContent::default();
        config.moving.bounds = [0.8, 0.1, 0.4, 0.5];
        assert!(config.validate().is_err());
    }

    #[test]
    fn attribution_literal_is_not_a_configurable_field() {
        let config = WatermarkConfig::default();
        let encoded = serde_json::to_value(config).unwrap();
        assert!(encoded["attribution"].get("text").is_none());
        assert!(encoded["attribution"].get("position").is_none());
        assert_eq!(
            ATTRIBUTION_TEXT,
            "\u{203b}Made with\u{300c}Sono-Renderer\u{300d}by Nexint."
        );
    }

    #[test]
    fn attribution_renders_inside_small_landscape_and_4_3_outputs() {
        for (width, height) in [(1280, 720), (1024, 768), (320, 180), (160, 90)] {
            let mut pixels = vec![32; width as usize * height as usize * 3];
            let config = WatermarkConfig::default();
            config.validate().unwrap();
            render_after_gameplay_ui(&mut pixels, width, height, &config, 0.0).unwrap();
            let changed: Vec<_> = pixels
                .chunks_exact(3)
                .enumerate()
                .filter_map(|(index, pixel)| (pixel != [32, 32, 32]).then_some(index))
                .collect();
            assert!(
                !changed.is_empty(),
                "attribution vanished at {width}x{height}"
            );
            assert_eq!(pixels.len(), width as usize * height as usize * 3);
            let coordinates: Vec<_> = changed
                .iter()
                .map(|index| (*index as u32 % width, *index as u32 / width))
                .collect();
            let margin = 10.0_f64
                .min(f64::from(width) * 0.25)
                .min(f64::from(height) * 0.25);
            let min_x = coordinates.iter().map(|(x, _)| *x).min().unwrap();
            let max_x = coordinates.iter().map(|(x, _)| *x).max().unwrap();
            let min_y = coordinates.iter().map(|(_, y)| *y).min().unwrap();
            let max_y = coordinates.iter().map(|(_, y)| *y).max().unwrap();
            assert!(f64::from(min_x) >= margin - 1.0);
            assert!(f64::from(max_x) < f64::from(width) - margin + 1.0);
            assert!(f64::from(max_y) < f64::from(height) - margin + 1.0);
            assert!(
                min_y > height * 3 / 4,
                "attribution was not bottom-anchored"
            );
        }
    }

    #[test]
    fn master_off_leaves_both_compositor_stages_pixel_identical() {
        let mut config = WatermarkConfig::default();
        config.enabled = false;
        config.moving.enabled = true;
        config.moving.movement = WatermarkMovement::RandomWaypoints {
            segment_seconds: 1.0,
            interpolation: WatermarkInterpolation::Linear,
        };
        let mut pixels = vec![12; 320 * 180 * 3];
        let before = pixels.clone();
        render_before_gameplay_ui(&mut pixels, 320, 180, &config, 10.0).unwrap();
        render_after_gameplay_ui(&mut pixels, 320, 180, &config, 10.0).unwrap();
        assert_eq!(pixels, before);
    }

    #[test]
    fn moving_image_asset_is_tinted_rotated_and_composited() {
        let mut config = WatermarkConfig::default();
        config
            .assets
            .insert("fixture".into(), PathBuf::from("fixture.png"));
        config.loaded_assets.insert(
            "fixture".into(),
            Arc::new(UiBitmap {
                width: 8,
                height: 4,
                rgba: Arc::new(vec![255; 8 * 4 * 4]),
            }),
        );
        config.moving.enabled = true;
        config.moving.content = WatermarkContent::Image {
            asset: "fixture".into(),
            source: [0.0, 0.0, 1.0, 1.0],
        };
        config.moving.image_height = 28.0;
        config.moving.color = [255, 40, 20, 255];
        config.moving.opacity = 0.5;
        config.moving.rotation_degrees = 17.0;
        let mut pixels = vec![0; 160 * 90 * 3];
        render_after_gameplay_ui(&mut pixels, 160, 90, &config, 0.0).unwrap();
        assert!(pixels.chunks_exact(3).any(|p| p[0] > p[1] && p[0] > 0));
    }

    #[test]
    fn configured_sparse_bitmap_font_uses_custom_and_fallback_glyphs() {
        let mut config = WatermarkConfig::default();
        config
            .assets
            .insert("one-pixel".into(), PathBuf::from("one.png"));
        config.loaded_assets.insert(
            "one-pixel".into(),
            Arc::new(UiBitmap {
                width: 1,
                height: 1,
                rgba: Arc::new(vec![255; 4]),
            }),
        );
        config.fonts.insert(
            "custom-sparse".into(),
            UiBitmapFont {
                asset: "one-pixel".into(),
                glyph_width: 1,
                glyph_height: 1,
                columns: 1,
                first_codepoint: 32,
                advance: 0.9,
                glyph_map: BTreeMap::from([("M".into(), 0)]),
                glyph_assets: BTreeMap::new(),
                glyph_scales: BTreeMap::new(),
                glyph_advances: BTreeMap::new(),
                center_spacing: None,
                glyph_max_dimension: None,
            },
        );
        config.attribution.font = Some("custom-sparse".into());
        for (font_size, width, height) in [(14.0, 640, 360), (24.0, 1024, 768), (9.0, 320, 180)] {
            config.attribution.font_size = font_size;
            let mut pixels = vec![0; width as usize * height as usize * 3];
            render_after_gameplay_ui(&mut pixels, width, height, &config, 0.0).unwrap();
            assert!(pixels.chunks_exact(3).any(|p| *p != [0, 0, 0]));
        }
    }
}
