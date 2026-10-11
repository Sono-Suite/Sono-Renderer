//! Versioned, data-driven presentation asset packs.
//!
//! Packs describe media and its relationships; the compositor consumes stable
//! logical IDs and never infers gameplay behavior from a filename. Paths are
//! resolved relative to the pack and are confined to its directory.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};

pub const ASSET_PACK_FORMAT: &str = "sono-renderer-presentation-asset-pack";
pub const ASSET_PACK_SCHEMA_VERSION: u32 = 1;
pub const MAX_ASSET_PACK_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_ASSET_COUNT: usize = 4096;
const MAX_ASSET_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 32768;
const MAX_IMAGE_PIXELS: u64 = 268_435_456;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PresentationAssetKind {
    Image,
    Sprite,
    Glyph,
    Video,
    Audio,
    Font,
    Composition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct AssetVariantSelector {
    /// Human-readable aspect label such as `16:9` or `4:3`.
    pub aspect_ratio: Option<String>,
    pub presentation_preset: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PresentationAssetVariant {
    pub source: PathBuf,
    pub selector: AssetVariantSelector,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for PresentationAssetVariant {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            selector: AssetVariantSelector::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PresentationAssetEntry {
    /// Stable, pack-local logical identity, independent of GUI element state.
    pub id: String,
    pub source: PathBuf,
    pub kind: PresentationAssetKind,
    pub family: Option<String>,
    pub character: Option<String>,
    pub language: Option<String>,
    pub variant_group: Option<String>,
    pub selector: AssetVariantSelector,
    pub variants: Vec<PresentationAssetVariant>,
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for PresentationAssetEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            source: PathBuf::new(),
            kind: PresentationAssetKind::Image,
            family: None,
            character: None,
            language: None,
            variant_group: None,
            selector: AssetVariantSelector::default(),
            variants: Vec::new(),
            tags: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PresentationAssetPack {
    pub format: String,
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub assets: Vec<PresentationAssetEntry>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for PresentationAssetPack {
    fn default() -> Self {
        Self {
            format: ASSET_PACK_FORMAT.into(),
            schema_version: ASSET_PACK_SCHEMA_VERSION,
            id: String::new(),
            name: String::new(),
            description: String::new(),
            assets: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InspectedPresentationAsset {
    pub id: String,
    pub source: PathBuf,
    pub kind: PresentationAssetKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub dimensions: Option<[u32; 2]>,
    pub has_alpha: Option<bool>,
    pub video: Option<VideoAssetMetadata>,
    pub variant_group: Option<String>,
    pub selector: AssetVariantSelector,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoAssetMetadata {
    pub width: u32,
    pub height: u32,
    pub frame_rate: String,
    pub video_codec: String,
    pub audio_codec: Option<String>,
    pub duration_seconds: f64,
}

impl PresentationAssetPack {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata = fs::metadata(path)
            .with_context(|| format!("accessing presentation asset pack {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_ASSET_PACK_BYTES {
            bail!("asset pack manifest must be a regular file no larger than {MAX_ASSET_PACK_BYTES} bytes");
        }
        let bytes = fs::read(path)
            .with_context(|| format!("reading presentation asset pack {}", path.display()))?;
        let pack: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing presentation asset pack {}", path.display()))?;
        pack.validate_schema()?;
        Ok(pack)
    }

    pub fn validate_schema(&self) -> Result<()> {
        if self.format != ASSET_PACK_FORMAT {
            bail!(
                "unsupported presentation asset-pack format {:?}",
                self.format
            );
        }
        if self.schema_version != ASSET_PACK_SCHEMA_VERSION {
            bail!(
                "unsupported presentation asset-pack schema version {}",
                self.schema_version
            );
        }
        if !valid_identity(&self.id) || self.name.trim().is_empty() {
            bail!("presentation asset pack requires a safe id and nonempty name");
        }
        if self.assets.len() > MAX_ASSET_COUNT {
            bail!("presentation asset pack exceeds the {MAX_ASSET_COUNT}-asset limit");
        }
        let mut ids = BTreeSet::new();
        for entry in &self.assets {
            if !valid_identity(&entry.id) || !ids.insert(entry.id.as_str()) {
                bail!(
                    "presentation asset pack contains an invalid or duplicate asset id {:?}",
                    entry.id
                );
            }
            validate_relative_source(&entry.source)?;
            for variant in &entry.variants {
                validate_relative_source(&variant.source)?;
            }
            if entry
                .selector
                .aspect_ratio
                .as_deref()
                .is_some_and(|v| !valid_aspect_label(v))
            {
                bail!("asset {:?} has an invalid aspect-ratio selector", entry.id);
            }
        }
        Ok(())
    }

    /// Resolve `pack://logical-id`, optionally selecting an explicitly labeled
    /// aspect variant. If no exact variant is declared, the base source is used.
    pub fn resolve_reference(
        &self,
        manifest_path: impl AsRef<Path>,
        reference: &str,
        aspect_ratio: Option<&str>,
    ) -> Result<PathBuf> {
        self.resolve_reference_with_root(manifest_path, reference, aspect_ratio, None)
    }

    /// Resolve a pack-local reference beneath an explicitly selected asset
    /// root. The override changes only the base directory: relative-path
    /// validation and canonical containment checks still apply.
    pub fn resolve_reference_with_root(
        &self,
        manifest_path: impl AsRef<Path>,
        reference: &str,
        aspect_ratio: Option<&str>,
        asset_root: Option<&Path>,
    ) -> Result<PathBuf> {
        let id = reference
            .strip_prefix("pack://")
            .context("asset reference must use pack://<logical-id>")?;
        let (id, uri_aspect) = id
            .split_once('@')
            .map_or((id, None), |(id, aspect)| (id, Some(aspect)));
        let aspect_ratio = aspect_ratio.or(uri_aspect);
        let entry = self
            .assets
            .iter()
            .find(|entry| entry.id == id)
            .with_context(|| format!("asset pack {:?} has no asset id {id:?}", self.id))?;
        let source = entry
            .variants
            .iter()
            .find(|variant| {
                aspect_ratio.is_some() && variant.selector.aspect_ratio.as_deref() == aspect_ratio
            })
            .map(|variant| &variant.source)
            .unwrap_or(&entry.source);
        self.resolve_source_with_root(manifest_path, source, asset_root)
    }

    pub fn resolve_source(
        &self,
        manifest_path: impl AsRef<Path>,
        source: &Path,
    ) -> Result<PathBuf> {
        self.resolve_source_with_root(manifest_path, source, None)
    }

    /// Resolve a pack-relative source against `asset_root` when supplied, or
    /// against the manifest directory otherwise. Both roots are canonicalized
    /// before joining so symlinks cannot escape the user-selected directory.
    pub fn resolve_source_with_root(
        &self,
        manifest_path: impl AsRef<Path>,
        source: &Path,
        asset_root: Option<&Path>,
    ) -> Result<PathBuf> {
        validate_relative_source(source)?;
        let manifest_path = manifest_path.as_ref();
        let root_path = asset_root
            .or_else(|| manifest_path.parent())
            .unwrap_or_else(|| Path::new("."));
        let root = root_path.canonicalize().with_context(|| {
            format!(
                "resolving presentation asset root {} for manifest {}",
                root_path.display(),
                manifest_path.display()
            )
        })?;
        let resolved = root.join(source);
        let canonical = resolved
            .canonicalize()
            .with_context(|| format!("resolving asset source {}", resolved.display()))?;
        if !canonical.starts_with(&root) {
            bail!(
                "asset source {} escapes its pack directory",
                source.display()
            );
        }
        let metadata = fs::metadata(&canonical)
            .with_context(|| format!("accessing asset source {}", canonical.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_ASSET_BYTES {
            bail!(
                "asset source {} is not a regular file within the 2 GiB limit",
                source.display()
            );
        }
        Ok(canonical)
    }

    /// Full, explicit inspection. Hashes reuse the renderer's shared SHA-256
    /// implementation; normal frame rendering does not pay this cost.
    pub fn inspect(
        &self,
        manifest_path: impl AsRef<Path>,
    ) -> Result<Vec<InspectedPresentationAsset>> {
        let manifest_path = manifest_path.as_ref();
        let mut result = Vec::with_capacity(self.assets.len());
        for entry in &self.assets {
            let path = self.resolve_source(manifest_path, &entry.source)?;
            result.push(inspect_entry(entry, &path)?);
            for variant in &entry.variants {
                let mut variant_entry = entry.clone();
                variant_entry.source = variant.source.clone();
                variant_entry.selector = variant.selector.clone();
                result.push(inspect_entry(
                    &variant_entry,
                    &self.resolve_source(manifest_path, &variant.source)?,
                )?);
            }
        }
        Ok(result)
    }

    pub fn inspect_with_ffprobe(
        &self,
        manifest_path: impl AsRef<Path>,
        ffprobe: impl AsRef<Path>,
    ) -> Result<Vec<InspectedPresentationAsset>> {
        let manifest_path = manifest_path.as_ref();
        let mut result = self.inspect(manifest_path)?;
        for inspected in &mut result {
            if inspected.kind != PresentationAssetKind::Video {
                continue;
            }
            let source = self.resolve_source(manifest_path, &inspected.source)?;
            inspected.video = Some(probe_video(ffprobe.as_ref(), &source)?);
        }
        Ok(result)
    }
}

fn probe_video(ffprobe: &Path, path: &Path) -> Result<VideoAssetMetadata> {
    let output = crate::export_control::hide_child_window(&mut Command::new(ffprobe))
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_name,codec_type,width,height,r_frame_rate:format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .with_context(|| format!("running FFprobe for presentation video {}", path.display()))?;
    if !output.status.success() {
        bail!(
            "FFprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json: Value = serde_json::from_slice(&output.stdout)
        .with_context(|| format!("parsing FFprobe metadata for {}", path.display()))?;
    let streams = json["streams"]
        .as_array()
        .context("FFprobe returned no stream list")?;
    let video = streams
        .iter()
        .find(|stream| stream["codec_type"] == "video")
        .context("presentation video has no video stream")?;
    let width = video["width"]
        .as_u64()
        .context("video stream has no width")? as u32;
    let height = video["height"]
        .as_u64()
        .context("video stream has no height")? as u32;
    let duration_seconds = json["format"]["duration"]
        .as_str()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value > 0.0)
        .context("video container has no finite positive duration")?;
    Ok(VideoAssetMetadata {
        width,
        height,
        frame_rate: video["r_frame_rate"]
            .as_str()
            .unwrap_or("unknown")
            .to_owned(),
        video_codec: video["codec_name"].as_str().unwrap_or("unknown").to_owned(),
        audio_codec: streams
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
            .and_then(|stream| stream["codec_name"].as_str())
            .map(str::to_owned),
        duration_seconds,
    })
}

fn inspect_entry(
    entry: &PresentationAssetEntry,
    path: &Path,
) -> Result<InspectedPresentationAsset> {
    let (sha256, size_bytes) = crate::content_hash::sha256_file(path)?;
    let (dimensions, has_alpha) = match entry.kind {
        PresentationAssetKind::Image
        | PresentationAssetKind::Sprite
        | PresentationAssetKind::Glyph => {
            let decoded = image::ImageReader::open(path)
                .with_context(|| format!("opening image asset {}", path.display()))?
                .with_guessed_format()
                .with_context(|| format!("identifying image asset {}", path.display()))?;
            let dimensions = decoded
                .into_dimensions()
                .with_context(|| format!("reading image dimensions for {}", path.display()))?;
            if dimensions.0 == 0
                || dimensions.1 == 0
                || dimensions.0 > MAX_IMAGE_DIMENSION
                || dimensions.1 > MAX_IMAGE_DIMENSION
                || u64::from(dimensions.0) * u64::from(dimensions.1) > MAX_IMAGE_PIXELS
            {
                bail!(
                    "image asset {} exceeds supported dimensions",
                    path.display()
                );
            }
            // PNG color type is represented by the file decoder; full alpha is
            // preserved by the production image loader. This flag is a format
            // capability, not a claim that any individual pixel is translucent.
            let color = image::ImageReader::open(path)
                .with_context(|| format!("opening image asset {}", path.display()))?
                .with_guessed_format()
                .with_context(|| format!("identifying image asset {}", path.display()))?
                .decode()
                .with_context(|| format!("decoding image asset {}", path.display()))?;
            (
                Some([dimensions.0, dimensions.1]),
                Some(color.color().has_alpha()),
            )
        }
        _ => (None, None),
    };
    Ok(InspectedPresentationAsset {
        id: entry.id.clone(),
        source: entry.source.clone(),
        kind: entry.kind,
        size_bytes,
        sha256,
        dimensions,
        has_alpha,
        video: None,
        variant_group: entry.variant_group.clone(),
        selector: entry.selector.clone(),
    })
}

fn validate_relative_source(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        bail!(
            "asset source must be a safe relative path without traversal: {}",
            path.display()
        );
    }
    Ok(())
}

fn valid_identity(value: &str) -> bool {
    !value.trim().is_empty()
        && value.len() <= 160
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+' | b'%')
        })
}

fn valid_aspect_label(value: &str) -> bool {
    value.split_once(':').is_some_and(|(w, h)| {
        w.parse::<u32>().is_ok_and(|n| n > 0) && h.parse::<u32>().is_ok_and(|n| n > 0)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn pack(source: PathBuf) -> PresentationAssetPack {
        PresentationAssetPack {
            id: "fixture".into(),
            name: "Fixture".into(),
            assets: vec![PresentationAssetEntry {
                id: "picture".into(),
                source,
                kind: PresentationAssetKind::Image,
                ..PresentationAssetEntry::default()
            }],
            ..PresentationAssetPack::default()
        }
    }

    #[test]
    fn asset_pack_inspection_reports_dimensions_alpha_and_shared_hash() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("sprite.png");
        image::RgbaImage::from_pixel(3, 2, image::Rgba([20, 30, 40, 80]))
            .save(&path)
            .unwrap();
        let manifest = dir.path().join("pack.json");
        let document = pack("sprite.png".into());
        fs::write(&manifest, serde_json::to_vec(&document).unwrap()).unwrap();
        let loaded = PresentationAssetPack::load(&manifest).unwrap();
        let inspected = loaded.inspect(&manifest).unwrap();
        assert_eq!(inspected.len(), 1);
        assert_eq!(inspected[0].dimensions, Some([3, 2]));
        assert_eq!(inspected[0].has_alpha, Some(true));
        assert_eq!(
            inspected[0].sha256,
            crate::content_hash::sha256_file(&path).unwrap().0
        );
    }

    #[test]
    fn unsafe_paths_duplicate_ids_and_future_versions_are_rejected() {
        let mut document = pack("../outside.png".into());
        assert!(document.validate_schema().is_err());
        document.assets[0].source = "safe.png".into();
        document.assets.push(document.assets[0].clone());
        assert!(document.validate_schema().is_err());
        document.assets.pop();
        document.schema_version += 1;
        assert!(document.validate_schema().is_err());
    }

    #[test]
    fn pack_reference_selects_declared_aspect_variant() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("wide.mp4"), b"base").unwrap();
        fs::write(dir.path().join("tall.mp4"), b"variant").unwrap();
        let manifest = dir.path().join("pack.json");
        let mut document = PresentationAssetPack {
            id: "fixture".into(),
            name: "Fixture".into(),
            assets: vec![PresentationAssetEntry {
                id: "clip".into(),
                source: "wide.mp4".into(),
                kind: PresentationAssetKind::Video,
                variants: vec![PresentationAssetVariant {
                    source: "tall.mp4".into(),
                    selector: AssetVariantSelector {
                        aspect_ratio: Some("4:3".into()),
                        ..AssetVariantSelector::default()
                    },
                    ..PresentationAssetVariant::default()
                }],
                ..PresentationAssetEntry::default()
            }],
            ..PresentationAssetPack::default()
        };
        fs::write(&manifest, serde_json::to_vec(&document).unwrap()).unwrap();
        document = PresentationAssetPack::load(&manifest).unwrap();
        assert_eq!(
            document
                .resolve_reference(&manifest, "pack://clip@4:3", None)
                .unwrap(),
            dir.path().join("tall.mp4").canonicalize().unwrap()
        );
        assert_eq!(
            document
                .resolve_reference(&manifest, "pack://clip@16:9", None)
                .unwrap(),
            dir.path().join("wide.mp4").canonicalize().unwrap()
        );
    }

    #[test]
    fn explicit_asset_root_supports_unicode_nested_paths_and_remains_confined() {
        let dir = tempdir().unwrap();
        let manifest_dir = dir.path().join("manifest");
        let asset_root = dir.path().join("素材 pack (test)");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::create_dir_all(asset_root.join("子 folder")).unwrap();
        fs::write(manifest_dir.join("pack.json"), b"{}").unwrap();
        fs::write(asset_root.join("子 folder").join("画像 one.png"), b"image").unwrap();
        let manifest = manifest_dir.join("pack.json");
        let document = pack(PathBuf::from("子 folder/画像 one.png"));

        let resolved = document
            .resolve_source_with_root(
                &manifest,
                Path::new("子 folder/画像 one.png"),
                Some(&asset_root),
            )
            .unwrap();
        assert_eq!(
            resolved,
            asset_root
                .join("子 folder")
                .join("画像 one.png")
                .canonicalize()
                .unwrap()
        );
        assert!(document
            .resolve_source_with_root(&manifest, Path::new("../outside.png"), Some(&asset_root))
            .is_err());
    }
}
