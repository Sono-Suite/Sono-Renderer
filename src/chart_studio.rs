//! Generic Sonolus LevelData authoring primitives and standalone export.
//!
//! This module edits the same `formats::LevelData` consumed by Watch. It does
//! not infer gameplay meaning from archetype names, drawing behavior, or SFX.

use crate::formats::{EnginePackage, LevelData, LevelEntity};
use anyhow::{bail, Context, Result};
use flate2::{Compression, GzBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub const AUTHORING_SCHEMA_FORMAT: &str = "sono-chart-authoring-schema";
pub const AUTHORING_SCHEMA_VERSION: u32 = 1;
pub const AUTHORING_SCHEMA_LIMIT: u64 = 1024 * 1024;
pub const MAX_SCHEMA_TEMPLATE_BYTES: usize = 64 * 1024;
pub const MAX_SCHEMA_JSON_DEPTH: usize = 32;
pub const MAX_SCHEMA_JSON_VALUES: usize = 8_192;
pub const MAX_RAW_FIELD_NODES: usize = 8_192;
pub const MAX_RAW_FIELD_DEPTH: usize = 64;
pub const MAX_RAW_FIELD_PATH_BYTES: usize = 4_096;
pub const MAX_RAW_FIELD_STRING_BYTES: usize = 65_536;
pub const MAX_LEVELDATA_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_SCHEMA_ARCHETYPES: usize = 4096;
pub const MAX_SCHEMA_PROPERTIES: usize = 16_384;
pub const MAX_UNDO_OPERATIONS: usize = 256;
/// Forward-compatible extension key used by RenderProject to persist the
/// editable canonical LevelData snapshot without changing render input fields.
pub const PROJECT_LEVEL_KEY: &str = "chartStudioLevelData";
pub const PROJECT_LEVEL_SNAPSHOT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectLevelSnapshot {
    pub schema_version: u32,
    pub level_data: LevelData,
    /// Chart Studio's authoring-only beat reference. It is intentionally not
    /// part of Sonolus LevelData and does not imply an engine BPM entity.
    #[serde(default)]
    pub beat_grid: Option<ChartBeatGrid>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChartBeatGrid {
    pub bpm: f64,
    /// Reference beat in the decoded audio's media timeline, in seconds.
    pub phase_media_seconds: f64,
    #[serde(default)]
    pub source_sha256: Option<String>,
}

impl ChartBeatGrid {
    pub fn validate(&self) -> Result<()> {
        if !self.bpm.is_finite() || !(20.0..=400.0).contains(&self.bpm) {
            bail!("reference-grid BPM must be finite and between 20 and 400");
        }
        if !self.phase_media_seconds.is_finite() || self.phase_media_seconds.abs() > 86_400.0 {
            bail!("reference-grid phase must be finite and within one day of media time zero");
        }
        if self.source_sha256.as_ref().is_some_and(|hash| {
            hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            bail!("reference-grid source identity must be a SHA-256 hex digest");
        }
        Ok(())
    }
}

pub fn project_level_snapshot(
    level: &LevelData,
    beat_grid: Option<&ChartBeatGrid>,
    previous: Option<&Value>,
) -> Result<Value> {
    let mut extra = previous
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    extra.remove("schema_version");
    extra.remove("level_data");
    extra.remove("beat_grid");
    if let Some(beat_grid) = beat_grid {
        beat_grid.validate()?;
    }
    serde_json::to_value(ProjectLevelSnapshot {
        schema_version: PROJECT_LEVEL_SNAPSHOT_VERSION,
        level_data: level.clone(),
        beat_grid: beat_grid.cloned(),
        extra,
    })
    .context("serializing versioned project LevelData snapshot")
}

pub fn read_project_level_snapshot(value: &Value) -> Result<LevelData> {
    Ok(read_project_chart_snapshot(value)?.level_data)
}

pub fn read_project_chart_snapshot(value: &Value) -> Result<ProjectLevelSnapshot> {
    let snapshot: ProjectLevelSnapshot =
        serde_json::from_value(value.clone()).context("decoding project LevelData snapshot")?;
    if snapshot.schema_version != PROJECT_LEVEL_SNAPSHOT_VERSION {
        bail!(
            "unsupported project LevelData snapshot schema {}; this renderer supports {}",
            snapshot.schema_version,
            PROJECT_LEVEL_SNAPSHOT_VERSION
        );
    }
    if let Some(beat_grid) = &snapshot.beat_grid {
        beat_grid.validate()?;
    }
    Ok(snapshot)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EngineIdentity {
    /// SHA-256 of the loaded engine's metadata, configuration, ROM, PlayData,
    /// and Watch data, independent of the archive/directory path.
    pub content_sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AuthoringSchema {
    pub format: String,
    pub schema_version: u32,
    pub engine: EngineIdentity,
    pub archetypes: Vec<ArchetypeAuthoringInfo>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ArchetypeAuthoringInfo {
    /// Must exactly match an archetype name exposed by the bound engine.
    pub id: String,
    pub label: Option<String>,
    pub category: Option<String>,
    /// Optional canonical entity `data` value used when creating this entity.
    /// It is inert JSON, never code, and is copied into LevelData directly.
    pub initial_data: Option<Value>,
    pub timing: Option<TimingDescriptor>,
    pub properties: Vec<PropertyAuthoringInfo>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TimingDescriptor {
    /// Explicit JSON Pointer; the editor never guesses a timing field.
    pub path: String,
    pub unit: TimingUnit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimingUnit {
    WatchSeconds,
    MediaSeconds,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PropertyAuthoringInfo {
    /// JSON Pointer into the entity's `data` value. This is descriptive only;
    /// the core editor does not execute expressions or mutate fields by name.
    pub path: String,
    pub label: Option<String>,
    pub description: Option<String>,
    pub minimum: Option<f64>,
    pub maximum: Option<f64>,
    /// Missing values are accepted unless the schema explicitly requires them.
    pub required: bool,
    pub enum_values: Vec<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl Default for AuthoringSchema {
    fn default() -> Self {
        Self {
            format: AUTHORING_SCHEMA_FORMAT.into(),
            schema_version: AUTHORING_SCHEMA_VERSION,
            engine: EngineIdentity {
                content_sha256: String::new(),
            },
            archetypes: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl Default for ArchetypeAuthoringInfo {
    fn default() -> Self {
        Self {
            id: String::new(),
            label: None,
            category: None,
            initial_data: None,
            timing: None,
            properties: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl Default for PropertyAuthoringInfo {
    fn default() -> Self {
        Self {
            path: String::new(),
            label: None,
            description: None,
            minimum: None,
            maximum: None,
            required: false,
            enum_values: Vec::new(),
            extra: serde_json::Map::new(),
        }
    }
}

impl AuthoringSchema {
    pub fn load_for_engine(path: &Path, engine: &EnginePackage) -> Result<Self> {
        let metadata = fs::metadata(path)
            .with_context(|| format!("reading authoring schema {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > AUTHORING_SCHEMA_LIMIT {
            bail!("authoring schema must be a regular file no larger than {AUTHORING_SCHEMA_LIMIT} bytes");
        }
        let bytes = fs::read(path)?;
        let schema: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing authoring schema {}", path.display()))?;
        schema.validate_for_engine(engine)?;
        Ok(schema)
    }

    pub fn validate_for_engine(&self, engine: &EnginePackage) -> Result<()> {
        let identity = engine_identity(engine)?;
        let names = engine.level_archetype_names();
        self.validate_for_identity(&identity, &names)
    }

    pub fn validate_for_identity(
        &self,
        identity: &EngineIdentity,
        archetype_names: &[&str],
    ) -> Result<()> {
        if self.format != AUTHORING_SCHEMA_FORMAT {
            bail!("unsupported authoring schema format {:?}", self.format);
        }
        if self.schema_version != AUTHORING_SCHEMA_VERSION {
            bail!(
                "unsupported authoring schema version {}; this renderer supports {}",
                self.schema_version,
                AUTHORING_SCHEMA_VERSION
            );
        }
        if self.engine.content_sha256.len() != 64
            || !self
                .engine
                .content_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            bail!("authoring schema engine content_sha256 must be 64 hexadecimal characters");
        }
        if self.archetypes.len() > MAX_SCHEMA_ARCHETYPES {
            bail!("authoring schema has too many archetypes");
        }
        if !self
            .engine
            .content_sha256
            .eq_ignore_ascii_case(&identity.content_sha256)
        {
            bail!(
                "authoring schema engine identity mismatch (schema {}, loaded {})",
                self.engine.content_sha256,
                identity.content_sha256
            );
        }
        let engine_archetypes: BTreeSet<&str> = archetype_names.iter().copied().collect();
        let mut seen = BTreeSet::new();
        let mut property_count = 0usize;
        for archetype in &self.archetypes {
            if archetype.id.trim().is_empty() || !engine_archetypes.contains(archetype.id.as_str())
            {
                bail!(
                    "authoring schema references unknown engine archetype {:?}",
                    archetype.id
                );
            }
            if !seen.insert(archetype.id.as_str()) {
                bail!("authoring schema repeats archetype {:?}", archetype.id);
            }
            check_label("archetype label", archetype.label.as_deref())?;
            check_label("archetype category", archetype.category.as_deref())?;
            if let Some(initial_data) = &archetype.initial_data {
                let encoded = serde_json::to_vec(initial_data)
                    .context("serializing authoring schema entity template")?;
                if encoded.len() > MAX_SCHEMA_TEMPLATE_BYTES {
                    bail!("authoring schema template for {:?} exceeds {MAX_SCHEMA_TEMPLATE_BYTES} bytes", archetype.id);
                }
                validate_bounded_json(initial_data, MAX_SCHEMA_JSON_DEPTH, MAX_SCHEMA_JSON_VALUES)
                    .with_context(|| {
                        format!("validating initial data for archetype {:?}", archetype.id)
                    })?;
            }
            if let Some(timing) = &archetype.timing {
                validate_json_pointer(&timing.path)?;
            }
            property_count = property_count.saturating_add(archetype.properties.len());
            if property_count > MAX_SCHEMA_PROPERTIES {
                bail!("authoring schema has too many property descriptors");
            }
            for property in &archetype.properties {
                validate_json_pointer(&property.path)?;
                check_label("property label", property.label.as_deref())?;
                check_label("property description", property.description.as_deref())?;
                if property.minimum.is_some_and(|value| !value.is_finite())
                    || property.maximum.is_some_and(|value| !value.is_finite())
                    || property
                        .minimum
                        .zip(property.maximum)
                        .is_some_and(|(min, max)| min > max)
                {
                    bail!(
                        "authoring schema property {:?} has invalid numeric bounds",
                        property.path
                    );
                }
                if property.enum_values.len() > 256
                    || property
                        .enum_values
                        .iter()
                        .any(|value| value.len() > 4096 || value.contains('\0'))
                {
                    bail!(
                        "authoring schema property {:?} has invalid enum values",
                        property.path
                    );
                }
                if let Some(initial_data) = &archetype.initial_data {
                    match json_pointer(initial_data, &property.path) {
                        Some(value) => {
                            validate_property_value(value, property).with_context(|| {
                                format!(
                                    "invalid initial value for {:?} at {}",
                                    archetype.id, property.path
                                )
                            })?
                        }
                        None if property.required => bail!(
                            "initial data for {:?} is missing required property {}",
                            archetype.id,
                            property.path
                        ),
                        None => {}
                    }
                }
            }
        }
        Ok(())
    }

    pub fn archetype_label<'a>(&'a self, id: &'a str) -> &'a str {
        self.archetype(id)
            .and_then(|entry| entry.label.as_deref())
            .unwrap_or(id)
    }

    pub fn archetype(&self, id: &str) -> Option<&ArchetypeAuthoringInfo> {
        self.archetypes.iter().find(|entry| entry.id == id)
    }

    /// Validate only conventions that this explicitly matching authoring
    /// schema declares. This is not engine or gameplay validation.
    pub fn validate_level(&self, level: &LevelData) -> Vec<String> {
        let mut findings = Vec::new();
        for (entity_index, entity) in level.entities.iter().enumerate() {
            let Some(archetype) = entity.archetype.as_str().and_then(|id| self.archetype(id))
            else {
                continue;
            };
            for property in &archetype.properties {
                match json_pointer(&entity.data, &property.path) {
                    Some(value) => {
                        if let Err(error) = validate_property_value(value, property) {
                            findings
                                .push(format!("entity {entity_index} {}: {error}", property.path));
                        }
                    }
                    None if property.required => findings.push(format!(
                        "entity {entity_index} is missing required schema field {}",
                        property.path
                    )),
                    None => {}
                }
            }
        }
        findings
    }
}

fn json_pointer<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() {
        Some(value)
    } else {
        value.pointer(pointer)
    }
}

pub fn json_pointer_mut<'a>(value: &'a mut Value, pointer: &str) -> Option<&'a mut Value> {
    if pointer.is_empty() {
        Some(value)
    } else {
        value.pointer_mut(pointer)
    }
}

pub fn set_schema_property_value(
    data: &mut Value,
    property: &PropertyAuthoringInfo,
    next: Value,
) -> Result<()> {
    validate_json_pointer(&property.path)?;
    let current = json_pointer(data, &property.path).with_context(|| {
        format!(
            "schema path {} is not present in the entity data",
            property.path
        )
    })?;
    if value_kind(current) != value_kind(&next) {
        bail!(
            "schema property {} cannot change JSON type from {} to {}",
            property.path,
            value_kind(current),
            value_kind(&next)
        );
    }
    validate_property_value(&next, property)
        .with_context(|| format!("value violates schema property {}", property.path))?;
    *json_pointer_mut(data, &property.path).with_context(|| {
        format!(
            "schema path {} is not present in the entity data",
            property.path
        )
    })? = next;
    Ok(())
}

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn validate_property_value(value: &Value, property: &PropertyAuthoringInfo) -> Result<()> {
    if property.required && value.is_null() {
        bail!("required property cannot be null");
    }
    if property.minimum.is_some() || property.maximum.is_some() {
        let number = value
            .as_f64()
            .with_context(|| "numeric bounds require a numeric JSON value")?;
        if property.minimum.is_some_and(|minimum| number < minimum)
            || property.maximum.is_some_and(|maximum| number > maximum)
        {
            bail!("numeric value {number} is outside the declared range");
        }
    }
    if !property.enum_values.is_empty() {
        let string = value
            .as_str()
            .with_context(|| "enum values require a string JSON value")?;
        if !property.enum_values.iter().any(|option| option == string) {
            bail!("value {string:?} is not in the declared enum");
        }
    }
    Ok(())
}

fn validate_bounded_json(value: &Value, max_depth: usize, max_values: usize) -> Result<()> {
    let mut pending = vec![(value, 0usize)];
    let mut count = 0usize;
    while let Some((value, depth)) = pending.pop() {
        count = count.saturating_add(1);
        if count > max_values {
            bail!("JSON value contains more than {max_values} nodes");
        }
        if depth > max_depth {
            bail!("JSON value exceeds the maximum nesting depth of {max_depth}");
        }
        match value {
            Value::Array(items) => pending.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(items) => {
                if items
                    .keys()
                    .any(|key| key.len() > 16_384 || key.contains('\0'))
                {
                    bail!("JSON object contains an invalid or oversized key");
                }
                pending.extend(items.values().map(|item| (item, depth + 1)));
            }
            Value::String(text) if text.len() > 65_536 || text.contains('\0') => {
                bail!("JSON string is invalid or exceeds 65536 bytes");
            }
            _ => {}
        }
    }
    Ok(())
}

/// Enumerate primitive JSON fields for the generic entity inspector. Paths
/// are RFC 6901 JSON Pointers into the supplied value. Empty containers are
/// returned as leaves so the UI can report them; callers must keep raw JSON
/// editing available for structural changes.
pub fn json_leaf_pointers(value: &Value) -> Result<Vec<String>> {
    let mut pending = vec![(String::new(), value, 0usize)];
    let mut leaves = Vec::new();
    let mut visited = 0usize;
    while let Some((pointer, value, depth)) = pending.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_RAW_FIELD_NODES {
            bail!("entity data is too large for structured inspection; use the raw JSON editor");
        }
        if depth > MAX_RAW_FIELD_DEPTH {
            bail!("entity data is nested too deeply for structured inspection; use the raw JSON editor");
        }
        if pointer.len() > MAX_RAW_FIELD_PATH_BYTES {
            bail!(
                "entity data path is too long for structured inspection; use the raw JSON editor"
            );
        }
        match value {
            Value::Array(items) if !items.is_empty() => {
                for (index, item) in items.iter().enumerate().rev() {
                    pending.push((format!("{pointer}/{index}"), item, depth + 1));
                }
            }
            Value::Object(items) if !items.is_empty() => {
                for (key, item) in items.iter().rev() {
                    let escaped = key.replace('~', "~0").replace('/', "~1");
                    pending.push((format!("{pointer}/{escaped}"), item, depth + 1));
                }
            }
            Value::String(text) if text.len() > MAX_RAW_FIELD_STRING_BYTES => {
                bail!(
                    "entity string is too large for structured inspection; use the raw JSON editor"
                );
            }
            _ => leaves.push(pointer),
        }
    }
    Ok(leaves)
}

fn check_label(kind: &str, value: Option<&str>) -> Result<()> {
    if value.is_some_and(|value| value.len() > 16_384 || value.contains('\0')) {
        bail!("authoring schema {kind} is invalid or too long");
    }
    Ok(())
}

fn validate_json_pointer(pointer: &str) -> Result<()> {
    if pointer.len() > 512
        || pointer
            .as_bytes()
            .iter()
            .filter(|byte| **byte == b'/')
            .count()
            > 16
    {
        bail!("authoring schema JSON pointer is too long or deeply nested");
    }
    if !pointer.is_empty() && !pointer.starts_with('/') {
        bail!("authoring schema property path must be a JSON Pointer");
    }
    if pointer.contains('~') {
        let mut chars = pointer.chars();
        while let Some(character) = chars.next() {
            if character == '~' && !matches!(chars.next(), Some('0' | '1')) {
                bail!("authoring schema JSON pointer has an invalid escape");
            }
        }
    }
    Ok(())
}

pub fn engine_identity(engine: &EnginePackage) -> Result<EngineIdentity> {
    // Hash semantic engine contents, including level-facing Play declarations
    // as well as runtime-facing Watch data. The archive path and ZIP bytes are
    // deliberately excluded.
    let bytes = serde_json::to_vec(&(
        &engine.metadata,
        &engine.configuration,
        &engine.rom,
        &engine.play,
        &engine.watch,
    ))
    .context("serializing engine identity")?;
    Ok(EngineIdentity {
        content_sha256: crate::content_hash::sha256_hex(&bytes),
    })
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LevelValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub entity_count: usize,
}

impl LevelValidation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

pub fn validate_level(level: &LevelData, engine: Option<&EnginePackage>) -> LevelValidation {
    let names: Vec<&str> = engine
        .map(EnginePackage::level_archetype_names)
        .unwrap_or_default();
    validate_level_with_archetypes(level, engine.map(|_| names.as_slice()))
}

pub fn validate_level_with_archetypes(
    level: &LevelData,
    archetype_names: Option<&[&str]>,
) -> LevelValidation {
    let mut report = LevelValidation {
        entity_count: level.entities.len(),
        ..LevelValidation::default()
    };
    if level.bgm_offset.is_some_and(|value| !value.is_finite()) {
        report
            .errors
            .push("bgmOffset must be a finite number".into());
    }
    let archetypes = archetype_names.map(|names| names.iter().copied().collect::<BTreeSet<_>>());
    for (index, entity) in level.entities.iter().enumerate() {
        let Some(name) = entity.archetype.as_str() else {
            report
                .errors
                .push(format!("entity {index} archetype must be a string"));
            continue;
        };
        if name.trim().is_empty() {
            report
                .errors
                .push(format!("entity {index} archetype cannot be empty"));
        } else if let Some(archetypes) = &archetypes {
            // These two host-generated timing entities are explicitly
            // recognized by the current shared Watch host.
            if !archetypes.contains(name) && !matches!(name, "#BPM_CHANGE" | "#TIMESCALE_CHANGE") {
                report.warnings.push(format!("entity {index} uses archetype {name:?}, which is not declared by the selected engine"));
            }
        }
    }
    report
}

#[derive(Debug, Clone, PartialEq)]
enum EditOperation {
    Insert {
        index: usize,
        entity: LevelEntity,
    },
    Delete {
        index: usize,
        entity: LevelEntity,
    },
    Replace {
        index: usize,
        before: LevelEntity,
        after: LevelEntity,
    },
    Move {
        from: usize,
        to: usize,
    },
    BgmOffset {
        before: Option<f64>,
        after: Option<f64>,
    },
    BeatGrid {
        before: Option<ChartBeatGrid>,
        after: Option<ChartBeatGrid>,
    },
}

#[derive(Debug, Clone)]
pub struct ChartDocument {
    pub level: LevelData,
    pub beat_grid: Option<ChartBeatGrid>,
    pub source: Option<PathBuf>,
    operations: Vec<EditOperation>,
    cursor: usize,
    saved_cursor: Option<usize>,
    revision: u64,
}

impl Default for ChartDocument {
    fn default() -> Self {
        Self::new(LevelData {
            bgm_offset: None,
            entities: Vec::new(),
            metadata: None,
            extra: serde_json::Map::new(),
        })
    }
}

impl ChartDocument {
    pub fn new(level: LevelData) -> Self {
        Self {
            level,
            beat_grid: None,
            source: None,
            operations: Vec::new(),
            cursor: 0,
            saved_cursor: Some(0),
            revision: 0,
        }
    }

    pub fn with_beat_grid(level: LevelData, beat_grid: Option<ChartBeatGrid>) -> Result<Self> {
        if let Some(beat_grid) = &beat_grid {
            beat_grid.validate()?;
        }
        Ok(Self {
            level,
            beat_grid,
            source: None,
            operations: Vec::new(),
            cursor: 0,
            saved_cursor: Some(0),
            revision: 0,
        })
    }

    pub fn is_dirty(&self) -> bool {
        self.saved_cursor != Some(self.cursor)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.operations.len()
    }

    pub fn mark_saved(&mut self, path: Option<PathBuf>) {
        self.saved_cursor = Some(self.cursor);
        if let Some(path) = path {
            self.source = Some(path);
        }
    }

    pub fn add_entity(&mut self, index: usize, archetype: impl Into<String>) -> Result<()> {
        let entity = LevelEntity {
            archetype: Value::String(archetype.into()),
            // Engines define their own entity-data shape. An empty array is a
            // neutral structural starting point and remains editable as JSON.
            data: Value::Array(Vec::new()),
            extra: serde_json::Map::new(),
        };
        self.execute(EditOperation::Insert { index, entity })
    }

    pub fn insert_entity(&mut self, index: usize, entity: LevelEntity) -> Result<()> {
        self.execute(EditOperation::Insert { index, entity })
    }

    pub fn duplicate_entity(&mut self, index: usize) -> Result<()> {
        let entity = self
            .level
            .entities
            .get(index)
            .with_context(|| format!("entity index {index} is out of range"))?
            .clone();
        self.execute(EditOperation::Insert {
            index: index + 1,
            entity,
        })
    }

    pub fn delete_entity(&mut self, index: usize) -> Result<()> {
        let entity = self
            .level
            .entities
            .get(index)
            .with_context(|| format!("entity index {index} is out of range"))?
            .clone();
        self.execute(EditOperation::Delete { index, entity })
    }

    pub fn replace_entity(&mut self, index: usize, entity: LevelEntity) -> Result<()> {
        let before = self
            .level
            .entities
            .get(index)
            .with_context(|| format!("entity index {index} is out of range"))?
            .clone();
        self.execute(EditOperation::Replace {
            index,
            before,
            after: entity,
        })
    }

    pub fn move_entity(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.level.entities.len() || to >= self.level.entities.len() {
            bail!("entity move index is out of range");
        }
        if from != to {
            self.execute(EditOperation::Move { from, to })?;
        }
        Ok(())
    }

    pub fn set_bgm_offset(&mut self, after: Option<f64>) -> Result<()> {
        if after.is_some_and(|value| !value.is_finite()) {
            bail!("bgmOffset must be finite");
        }
        let before = self.level.bgm_offset;
        if before != after {
            self.execute(EditOperation::BgmOffset { before, after })?;
        }
        Ok(())
    }

    /// Apply an explicitly reviewed authoring grid without touching generic
    /// engine-defined entity timing or the LevelData `bgmOffset` field.
    pub fn set_beat_grid(&mut self, after: Option<ChartBeatGrid>) -> Result<()> {
        if let Some(grid) = &after {
            grid.validate()?;
        }
        let before = self.beat_grid.clone();
        if before != after {
            self.execute(EditOperation::BeatGrid { before, after })?;
        }
        Ok(())
    }

    pub fn undo(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        self.cursor -= 1;
        Self::apply(
            &mut self.level,
            &mut self.beat_grid,
            &self.operations[self.cursor],
            false,
        );
        self.revision = self.revision.wrapping_add(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.cursor == self.operations.len() {
            return false;
        }
        Self::apply(
            &mut self.level,
            &mut self.beat_grid,
            &self.operations[self.cursor],
            true,
        );
        self.cursor += 1;
        self.revision = self.revision.wrapping_add(1);
        true
    }

    fn execute(&mut self, operation: EditOperation) -> Result<()> {
        validate_operation(&self.level, &operation)?;
        self.operations.truncate(self.cursor);
        if self.saved_cursor.is_some_and(|saved| saved > self.cursor) {
            self.saved_cursor = None;
        }
        Self::apply(&mut self.level, &mut self.beat_grid, &operation, true);
        self.operations.push(operation);
        self.cursor += 1;
        self.revision = self.revision.wrapping_add(1);
        if self.operations.len() > MAX_UNDO_OPERATIONS {
            self.operations.remove(0);
            self.cursor -= 1;
            self.saved_cursor = self.saved_cursor.and_then(|saved| saved.checked_sub(1));
        }
        Ok(())
    }

    fn apply(
        level: &mut LevelData,
        beat_grid: &mut Option<ChartBeatGrid>,
        operation: &EditOperation,
        forward: bool,
    ) {
        match operation {
            EditOperation::Insert { index, entity } if forward => {
                level.entities.insert(*index, entity.clone())
            }
            EditOperation::Insert { index, .. } => {
                level.entities.remove(*index);
            }
            EditOperation::Delete { index, entity: _ } if forward => {
                level.entities.remove(*index);
            }
            EditOperation::Delete { index, entity } => {
                level.entities.insert(*index, entity.clone())
            }
            EditOperation::Replace { index, after, .. } if forward => {
                level.entities[*index] = after.clone()
            }
            EditOperation::Replace { index, before, .. } => level.entities[*index] = before.clone(),
            EditOperation::Move { from, to } if forward => move_entity_unchecked(level, *from, *to),
            EditOperation::Move { from, to } => move_entity_unchecked(level, *to, *from),
            EditOperation::BgmOffset { after, .. } if forward => level.bgm_offset = *after,
            EditOperation::BgmOffset { before, .. } => level.bgm_offset = *before,
            EditOperation::BeatGrid { after, .. } if forward => *beat_grid = after.clone(),
            EditOperation::BeatGrid { before, .. } => *beat_grid = before.clone(),
        }
    }
}

fn validate_operation(level: &LevelData, operation: &EditOperation) -> Result<()> {
    match operation {
        EditOperation::Insert { index, .. } if *index <= level.entities.len() => Ok(()),
        EditOperation::Delete { index, .. } | EditOperation::Replace { index, .. }
            if *index < level.entities.len() =>
        {
            Ok(())
        }
        EditOperation::Move { from, to }
            if *from < level.entities.len() && *to < level.entities.len() =>
        {
            Ok(())
        }
        EditOperation::BgmOffset { .. } => Ok(()),
        EditOperation::BeatGrid { .. } => Ok(()),
        _ => bail!("chart edit index is out of range"),
    }
}

fn move_entity_unchecked(level: &mut LevelData, from: usize, to: usize) {
    let entity = level.entities.remove(from);
    level.entities.insert(to, entity);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LevelExportReport {
    pub path: PathBuf,
    pub sha256: String,
    pub json_bytes: usize,
    pub gzip_bytes: usize,
    pub entities: usize,
}

/// Write deterministic gzip-compressed canonical LevelData with atomic
/// replacement. `overwrite` must reflect an explicit user decision.
pub fn export_level_gzip(
    level: &LevelData,
    path: &Path,
    overwrite: bool,
) -> Result<LevelExportReport> {
    let validation = validate_level(level, None);
    if !validation.is_valid() {
        bail!(
            "LevelData validation failed: {}",
            validation.errors.join("; ")
        );
    }
    if path.as_os_str().is_empty() {
        bail!("LevelData export path is empty");
    }
    if path.exists() && !overwrite {
        bail!(
            "output already exists; confirm overwrite explicitly: {}",
            path.display()
        );
    }
    let json = serde_json::to_vec(level).context("serializing canonical LevelData")?;
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)
        .with_context(|| format!("creating LevelData output directory {}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)
        .with_context(|| format!("creating temporary export in {}", parent.display()))?;
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(temporary.as_file_mut(), Compression::default());
    encoder
        .write_all(&json)
        .context("compressing LevelData JSON")?;
    let output = encoder
        .finish()
        .context("finishing LevelData gzip stream")?;
    output.flush()?;
    temporary
        .as_file()
        .sync_all()
        .context("syncing temporary LevelData export")?;
    // Run the actual production payload parser on the temporary artifact
    // before it can replace a previous valid output.
    let round_trip = crate::formats::load_level_data(temporary.path())
        .context("validating exported gzip with the production LevelData parser")?;
    if round_trip != *level {
        bail!("LevelData export did not preserve canonical semantics");
    }
    if path.exists() && !overwrite {
        bail!(
            "output appeared during export; refusing to overwrite {}",
            path.display()
        );
    }
    let gzip_bytes = fs::read(temporary.path())?;
    let sha256 = crate::content_hash::sha256_hex(&gzip_bytes);
    let persisted = if overwrite {
        temporary.persist(path)
    } else {
        temporary.persist_noclobber(path)
    };
    persisted
        .map_err(|error| error.error)
        .with_context(|| format!("atomically publishing LevelData output {}", path.display()))?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(LevelExportReport {
        path: path.to_path_buf(),
        sha256,
        json_bytes: json.len(),
        gzip_bytes: gzip_bytes.len(),
        entities: level.entities.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::formats;
    use std::io::Read;

    fn empty_level() -> LevelData {
        ChartDocument::default().level
    }

    fn entity(name: &str) -> LevelEntity {
        LevelEntity {
            archetype: Value::String(name.into()),
            data: serde_json::json!([{"name":"#TIME","value":1.25},{"name":"custom","value":{"x":2}}]),
            extra: serde_json::json!({"vendorExtra":true})
                .as_object()
                .unwrap()
                .clone(),
        }
    }

    #[test]
    fn generic_edits_duplicate_and_undo_redo_keep_canonical_level_data() {
        let mut document = ChartDocument::new(LevelData {
            bgm_offset: Some(-0.25),
            entities: vec![entity("Arbitrary")],
            metadata: Some(serde_json::json!({"unknownMetadata":7})),
            extra: serde_json::json!({"unknownRoot":{"keep":true}})
                .as_object()
                .unwrap()
                .clone(),
        });
        let original = document.level.clone();
        document.duplicate_entity(0).unwrap();
        assert!(document.is_dirty());
        assert_eq!(document.level.entities.len(), 2);
        document.delete_entity(0).unwrap();
        assert_eq!(document.level.entities.len(), 1);
        assert!(document.undo());
        assert_eq!(document.level.entities.len(), 2);
        assert!(document.undo());
        assert_eq!(document.level, original);
        assert!(!document.is_dirty());
        assert!(document.redo());
        assert_eq!(document.level.entities.len(), 2);
    }

    #[test]
    fn export_is_deterministic_gzip_and_round_trips_unknown_data_through_loader() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("level.json.gz");
        let second = directory.path().join("second.json.gz");
        let mut level = empty_level();
        level.bgm_offset = Some(0.125);
        level
            .extra
            .insert("futureRoot".into(), serde_json::json!({"opaque":[1,2,3]}));
        level.entities.push(entity("engine-provided-name"));
        let a = export_level_gzip(&level, &first, false).unwrap();
        let b = export_level_gzip(&level, &second, false).unwrap();
        assert_eq!(a.sha256, b.sha256);
        assert_eq!(formats::load_level(&first).unwrap(), level);
        let raw = fs::read(&first).unwrap();
        assert_eq!(&raw[..2], &[0x1f, 0x8b]);
        let mut decoder = flate2::read::GzDecoder::new(raw.as_slice());
        let mut json = Vec::new();
        decoder.read_to_end(&mut json).unwrap();
        assert_eq!(serde_json::from_slice::<LevelData>(&json).unwrap(), level);
    }

    #[test]
    fn export_refuses_implicit_overwrite_and_preserves_previous_valid_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("level.json.gz");
        let original = empty_level();
        export_level_gzip(&original, &path, false).unwrap();
        let before = fs::read(&path).unwrap();
        let mut changed = original.clone();
        changed.entities.push(entity("new"));
        assert!(export_level_gzip(&changed, &path, false).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        export_level_gzip(&changed, &path, true).unwrap();
        assert_eq!(formats::load_level(&path).unwrap(), changed);
    }

    #[test]
    fn schema_rejects_wrong_engine_identity_and_bounded_pointer_errors() {
        let engine: EnginePackage = serde_json::from_value(serde_json::json!({
            "root":"synthetic-engine",
            "metadata":{"version":1,"title":{"en":"Generic"}},
            "configuration":{},
            "rom":[],
            "play":{"archetypes":[{"name":"GenericEntity"}]},
            "watch":{"archetypes":[{"name":"GenericEntity"}],"nodes":[]}
        }))
        .unwrap();
        let identity = engine_identity(&engine).unwrap();
        let schema = AuthoringSchema {
            engine: identity,
            archetypes: vec![ArchetypeAuthoringInfo {
                id: engine.level_archetype_names()[0].to_owned(),
                label: Some("Human label".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(schema.validate_for_engine(&engine).is_ok());
        let mut changed_play = engine.clone();
        changed_play.play.as_mut().unwrap().archetypes[0].name = "RenamedEntity".into();
        assert_ne!(
            engine_identity(&changed_play).unwrap(),
            engine_identity(&engine).unwrap(),
            "PlayData changes must invalidate content-bound authoring metadata"
        );
        assert!(schema.validate_for_engine(&changed_play).is_err());
        let legacy_schema: AuthoringSchema = serde_json::from_value(serde_json::json!({
            "format": AUTHORING_SCHEMA_FORMAT,
            "schema_version": AUTHORING_SCHEMA_VERSION,
            "engine": {"content_sha256": schema.engine.content_sha256.clone()},
            "archetypes": [{"id":"GenericEntity","properties":[{"path":"/x"}],"legacySchemaNote":"kept"}],
            "futureSchemaField": {"opaque":true}
        }))
        .unwrap();
        assert!(legacy_schema.validate_for_engine(&engine).is_ok());
        assert!(legacy_schema.archetypes[0].initial_data.is_none());
        assert!(!legacy_schema.archetypes[0].properties[0].required);
        assert_eq!(legacy_schema.extra["futureSchemaField"]["opaque"], true);
        assert_eq!(
            legacy_schema.archetypes[0].extra["legacySchemaNote"],
            "kept"
        );
        let mut wrong = schema;
        wrong.engine.content_sha256 = "00".repeat(32);
        assert!(wrong.validate_for_engine(&engine).is_err());
        assert!(validate_json_pointer("/data/~2field").is_err());
    }

    #[test]
    fn generic_field_paths_preserve_unusual_keys_arrays_and_negative_values() {
        let data = serde_json::json!({
            "odd/name": -3.25,
            "tilde~key": [true, {"mystery": "opaque"}],
            "empty": {},
            "link": "entity:future"
        });
        let paths = json_leaf_pointers(&data).unwrap();
        assert!(paths.contains(&"/odd~1name".into()));
        assert!(paths.contains(&"/tilde~0key/0".into()));
        assert!(paths.contains(&"/tilde~0key/1/mystery".into()));
        assert!(paths.contains(&"/empty".into()));
        assert!(paths.contains(&"/link".into()));
        assert_eq!(
            data.pointer("/odd~1name").and_then(Value::as_f64),
            Some(-3.25)
        );
    }

    #[test]
    fn raw_field_inspector_falls_back_when_adversarial_json_exceeds_bounds() {
        let mut deep = Value::Null;
        for _ in 0..=MAX_RAW_FIELD_DEPTH {
            deep = serde_json::json!([deep]);
        }
        assert!(json_leaf_pointers(&deep)
            .unwrap_err()
            .to_string()
            .contains("nested too deeply"));

        let wide = Value::Array(vec![Value::Null; MAX_RAW_FIELD_NODES + 1]);
        assert!(json_leaf_pointers(&wide)
            .unwrap_err()
            .to_string()
            .contains("too large"));

        let wide_string = Value::String("x".repeat(MAX_RAW_FIELD_STRING_BYTES + 1));
        assert!(json_leaf_pointers(&wide_string)
            .unwrap_err()
            .to_string()
            .contains("string is too large"));
    }

    #[test]
    fn declarative_schema_templates_constraints_and_edits_use_canonical_undo() {
        let engine: EnginePackage = serde_json::from_value(serde_json::json!({
            "root":"synthetic-engine",
            "metadata":{"version":1,"title":{"en":"Synthetic"}},
            "configuration":{},
            "rom":[],
            "play":{"archetypes":[{"name":"Mysterious"}]},
            "watch":{"archetypes":[{"name":"Mysterious"}],"nodes":[]}
        }))
        .unwrap();
        let schema = AuthoringSchema {
            engine: engine_identity(&engine).unwrap(),
            archetypes: vec![ArchetypeAuthoringInfo {
                id: "Mysterious".into(),
                label: Some("Descriptive label only".into()),
                initial_data: Some(
                    serde_json::json!({"u":-2.5,"axis":"vertical","link":"entity:later"}),
                ),
                properties: vec![
                    PropertyAuthoringInfo {
                        path: "/u".into(),
                        label: Some("Declared scalar".into()),
                        minimum: Some(-4.0),
                        maximum: Some(4.0),
                        required: true,
                        ..Default::default()
                    },
                    PropertyAuthoringInfo {
                        path: "/axis".into(),
                        enum_values: vec!["horizontal".into(), "vertical".into()],
                        required: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        schema.validate_for_engine(&engine).unwrap();
        let info = schema.archetype("Mysterious").unwrap();
        let mut document = ChartDocument::new(empty_level());
        let initial = LevelEntity {
            archetype: Value::String(info.id.clone()),
            data: info.initial_data.clone().unwrap(),
            extra: Default::default(),
        };
        document.insert_entity(0, initial.clone()).unwrap();
        assert!(schema.validate_level(&document.level).is_empty());

        let mut edited = initial.clone();
        let number = serde_json::json!(1.25);
        set_schema_property_value(&mut edited.data, &info.properties[0], number).unwrap();
        assert!(set_schema_property_value(
            &mut edited.data,
            &info.properties[0],
            serde_json::json!(99.0)
        )
        .is_err());
        let axis = serde_json::json!("horizontal");
        set_schema_property_value(&mut edited.data, &info.properties[1], axis).unwrap();
        assert_eq!(edited.data["link"], "entity:later");
        document.replace_entity(0, edited.clone()).unwrap();
        assert!(schema.validate_level(&document.level).is_empty());
        assert!(document.undo());
        assert_eq!(document.level.entities[0], initial);
        assert!(document.redo());
        assert_eq!(document.level.entities[0], edited);

        let mut invalid = document.level.clone();
        invalid.entities[0].data["axis"] = serde_json::json!("diagonal");
        invalid.entities[0]
            .data
            .as_object_mut()
            .unwrap()
            .remove("u");
        let findings = schema.validate_level(&invalid);
        assert_eq!(findings.len(), 2);
        assert!(findings.iter().any(|finding| finding.contains("required")));
        assert!(findings.iter().any(|finding| finding.contains("enum")));
    }

    #[test]
    fn schema_rejects_oversized_or_constraint_violating_initial_templates() {
        let engine: EnginePackage = serde_json::from_value(serde_json::json!({
            "root":"synthetic-engine",
            "metadata":{"version":1},"configuration":{},"rom":[],
            "play":{"archetypes":[{"name":"Mysterious"}]},
            "watch":{"archetypes":[{"name":"Mysterious"}],"nodes":[]}
        }))
        .unwrap();
        let identity = engine_identity(&engine).unwrap();
        let mut schema = AuthoringSchema {
            engine: identity,
            archetypes: vec![ArchetypeAuthoringInfo {
                id: "Mysterious".into(),
                initial_data: Some(serde_json::json!({"value":8})),
                properties: vec![PropertyAuthoringInfo {
                    path: "/value".into(),
                    maximum: Some(4.0),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert!(schema.validate_for_engine(&engine).is_err());
        schema.archetypes[0].properties.clear();
        schema.archetypes[0].initial_data =
            Some(Value::String("x".repeat(MAX_SCHEMA_TEMPLATE_BYTES + 1)));
        assert!(schema.validate_for_engine(&engine).is_err());
    }

    #[test]
    fn exported_real_level_runs_through_the_authoritative_watch_runtime() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine =
            formats::load_engine(&repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"))
                .unwrap();
        let level_path = repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz");
        let level = formats::load_level(&level_path).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let exported = directory.path().join("level.json.gz");
        export_level_gzip(&level, &exported, false).unwrap();
        let reloaded = formats::load_level(&exported).unwrap();
        assert_eq!(reloaded, level);
        let mut runtime =
            crate::watch_runtime::WatchRuntime::new(&engine.watch, &reloaded).unwrap();
        runtime.set_parallel_updates(false, Some(1)).unwrap();
        let report = runtime.frame(0.0).unwrap();
        assert!(report.timeline.is_some());
        assert!(report.runtime_entity_count > 0);
    }

    #[test]
    fn structurally_edit_roundtrip_and_watch_preview_on_materially_different_real_engines() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
        let fixtures = [
            (
                "Horizon",
                repo.join("TestingSuite/Horizon/Horizon.zip"),
                repo.join("TestingSuite/Horizon/dreamer.json.gz"),
                repo.join("TestingSuite/Horizon/project(1).scp"),
            ),
            (
                "Next RUSH",
                repo.join("TestingSuite/Next RUSH/engine/Next RUSH.zip"),
                repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
                repo.join("TestingSuite/Next Sekai Engine/levels/larp 64x/ProSeka Faithful 0.8.4.scp"),
            ),
            (
                "Next Sekai",
                repo.join("TestingSuite/Next Sekai Engine/engine/Next Sekai Engine.zip"),
                repo.join("TestingSuite/Next Sekai Engine/levels/Various Artists - Baumkuchen x Retry Now/Baumkuchen x Retry Now.json.gz"),
                repo.join("TestingSuite/Next Sekai Engine/levels/larp 64x/ProSeka Faithful 0.8.4.scp"),
            ),
        ];
        let mut archetype_counts = Vec::new();
        for (engine_name, engine_path, level_path, resources_path) in fixtures {
            let engine = formats::load_engine(&engine_path).unwrap();
            let original = formats::load_level(&level_path).unwrap();
            assert!(
                original.entities.len() > 100,
                "{engine_name} fixture should be a real sizeable level"
            );
            let level_archetypes = engine.level_archetype_names();
            assert!(
                !level_archetypes.is_empty(),
                "{engine_name} EnginePlayData should declare level archetypes"
            );
            archetype_counts.push(level_archetypes.len());

            let clean_directory = tempfile::tempdir().unwrap();
            let clean_export = clean_directory.path().join("clean-level.json.gz");
            export_level_gzip(&original, &clean_export, false).unwrap();
            let clean_reimport = formats::load_level_data(&clean_export).unwrap();
            assert_eq!(
                clean_reimport, original,
                "{engine_name} no-edit production export/reimport"
            );

            // Empty LevelData plus an engine-declared archetype is a raw
            // structural creation operation. Runtime-required entity data is
            // deliberately not guessed or fabricated here.
            let mut scratch = ChartDocument::default();
            let archetype = level_archetypes[0].to_owned();
            scratch.add_entity(0, archetype.clone()).unwrap();
            assert_eq!(scratch.level.entities[0].archetype, archetype);
            assert_eq!(scratch.level.entities[0].data, Value::Array(Vec::new()));
            scratch.duplicate_entity(0).unwrap();
            scratch.delete_entity(0).unwrap();
            assert!(scratch.undo());
            assert!(scratch.undo());
            assert_eq!(scratch.level.entities.len(), 1);
            assert!(scratch.undo());
            assert!(scratch.level.entities.is_empty());

            // The schema-free editor deliberately treats every entity payload
            // as opaque JSON. This probe stores an uninterpreted extension and
            // does not claim a gameplay meaning for any entity field.
            let mut document = ChartDocument::new(original.clone());
            let mut schema_validation = None;
            let mut schema_entity_index = None;
            let schema_edit_count = if engine_name == "Horizon" {
                // This is an externally supplied authoring contract bound to
                // the real engine contents. It demonstrates that the generic
                // property controls can describe an engine field without
                // teaching Chart Studio what that field means.
                let schema = AuthoringSchema {
                    engine: engine_identity(&engine).unwrap(),
                    archetypes: vec![ArchetypeAuthoringInfo {
                        id: "RotateEvent".into(),
                        label: Some("Schema-described event".into()),
                        category: Some("External authoring schema test".into()),
                        properties: vec![PropertyAuthoringInfo {
                            path: "/1/value".into(),
                            label: Some("Schema-declared numeric field".into()),
                            description: Some(
                                "The editor uses this author-provided field contract; it does not infer engine meaning from the data".into(),
                            ),
                            minimum: Some(-1000.0),
                            maximum: Some(1000.0),
                            required: true,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                };
                schema.validate_for_engine(&engine).unwrap();
                let schema_index = original
                    .entities
                    .iter()
                    .position(|entity| entity.archetype == "RotateEvent")
                    .expect("Horizon fixture contains a RotateEvent entity");
                let mut entity = original.entities[schema_index].clone();
                let property = &schema.archetype("RotateEvent").unwrap().properties[0];
                set_schema_property_value(&mut entity.data, property, serde_json::json!(-1.25))
                    .unwrap();
                document.replace_entity(schema_index, entity).unwrap();
                schema_validation = Some(schema);
                schema_entity_index = Some(schema_index);
                1
            } else {
                0
            };
            let mut first_entity = original.entities[0].clone();
            first_entity.extra.insert(
                "genericityRoundTripProbe".into(),
                serde_json::json!({"negative":-17.25,"ref":"uninterpreted:future"}),
            );
            document.replace_entity(0, first_entity).unwrap();
            let old_offset = original.bgm_offset.unwrap_or(0.0);
            document.set_bgm_offset(Some(old_offset + 0.125)).unwrap();
            let expected = document.level.clone();
            if let Some(schema) = &schema_validation {
                assert!(schema.validate_level(&expected).is_empty());
                let schema_index = schema_entity_index.unwrap();
                assert_eq!(
                    expected.entities[schema_index].data.pointer("/1/value"),
                    Some(&serde_json::json!(-1.25))
                );
            }
            assert_eq!(expected.entities[0].data, original.entities[0].data);
            for (index, (actual, untouched)) in
                expected.entities.iter().zip(&original.entities).enumerate()
            {
                if index != 0 && Some(index) != schema_entity_index {
                    assert_eq!(actual, untouched, "unmodified entity {index}");
                }
            }
            for _ in 0..(schema_edit_count + 2) {
                assert!(document.undo());
            }
            assert_eq!(document.level, original);
            for _ in 0..(schema_edit_count + 2) {
                assert!(document.redo());
            }
            assert_eq!(document.level, expected);

            let directory = tempfile::tempdir().unwrap();
            let exported = directory.path().join("standalone-level.json.gz");
            export_level_gzip(&document.level, &exported, false).unwrap();
            let reimported = formats::load_level_data(&exported).unwrap();
            assert_eq!(
                reimported, expected,
                "{engine_name} production LevelData round trip"
            );
            for (index, (actual, untouched)) in reimported
                .entities
                .iter()
                .zip(&original.entities)
                .enumerate()
            {
                if index != 0 && Some(index) != schema_entity_index {
                    assert_eq!(actual, untouched, "round-trip entity {index}");
                }
            }

            // This constructs the same production WatchRuntime used by
            // renderer workflows; no editor-only evaluator is involved.
            let mut runtime = crate::watch_runtime::WatchRuntime::new(&engine.watch, &reimported)
                .unwrap_or_else(|error| {
                    panic!("{engine_name} Watch runtime rejected reimported LevelData: {error:#}")
                });
            runtime.set_parallel_updates(false, Some(1)).unwrap();
            let frame = runtime
                .frame(0.0)
                .unwrap_or_else(|error| panic!("{engine_name} Watch frame failed: {error:#}"));
            assert!(
                frame.timeline.is_some(),
                "{engine_name} runtime frame should produce canonical timeline state"
            );

            // Use the same production frame renderer as Chart Studio preview
            // with only the canonical exported LevelData and real resources.
            let mut render = crate::render::RenderConfig::default();
            render.engine = engine_path;
            render.resources = resources_path;
            render.level = exported;
            render.width = 320;
            render.height = 180;
            render.backend = crate::render::RenderBackend::Cpu;
            render.parallel_watch_updates = false;
            let rendered = render.render_frame(0.0).unwrap_or_else(|error| {
                panic!("{engine_name} production renderer rejected edited LevelData: {error:#}")
            });
            assert_eq!(
                rendered.rgb.len(),
                320 * 180 * 3,
                "{engine_name} frame size"
            );
        }
        assert_ne!(archetype_counts[0], archetype_counts[1]);
        assert!(archetype_counts[1] >= archetype_counts[0] * 4);
        assert_ne!(archetype_counts[1], archetype_counts[2]);
    }

    #[test]
    fn bounded_authoring_loader_stops_gzip_expansion_over_limit() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("oversized.json.gz");
        let mut encoder = GzBuilder::new()
            .mtime(0)
            .write(Vec::new(), Compression::default());
        encoder.write_all(&vec![b' '; 4096]).unwrap();
        fs::write(&path, encoder.finish().unwrap()).unwrap();
        assert!(formats::load_level_data_bounded(&path, 128)
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
    }

    #[test]
    fn unknown_fields_survive_json_edit_roundtrip() {
        let raw = serde_json::json!({
            "bgmOffset": 0.5,
            "entities": [{"archetype":"Odd","data":{"private":[1,"x"]},"futureEntityFlag":{"a":true}}],
            "futureDocumentField": {"opaque": "yes"}
        });
        let level: LevelData = serde_json::from_value(raw.clone()).unwrap();
        let exported = serde_json::to_value(level).unwrap();
        assert_eq!(exported, raw);
    }

    #[test]
    fn existing_render_project_persists_the_canonical_chart_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("chart.sonoproject");
        let level = LevelData {
            entities: vec![entity("ArbitraryEngineArchetype")],
            extra: serde_json::json!({"vendorField":"kept"})
                .as_object()
                .unwrap()
                .clone(),
            ..empty_level()
        };
        let grid = ChartBeatGrid {
            bpm: 128.5,
            phase_media_seconds: 0.137,
            source_sha256: Some("cd".repeat(32)),
        };
        let mut project = crate::project::RenderProject::default();
        project.extra.insert(
            PROJECT_LEVEL_KEY.into(),
            project_level_snapshot(&level, Some(&grid), None).unwrap(),
        );
        project.save(&path).unwrap();
        let reopened = crate::project::RenderProject::load(&path).unwrap();
        let restored = read_project_chart_snapshot(&reopened.extra[PROJECT_LEVEL_KEY]).unwrap();
        assert_eq!(restored.level_data, level);
        assert_eq!(restored.beat_grid, Some(grid));
    }

    #[test]
    fn accepted_reference_grid_is_undoable_and_round_trips_without_retiming_level_data() {
        let mut level = empty_level();
        level.bgm_offset = Some(0.25);
        level.entities.push(entity("engine-defined"));
        let before = level.clone();
        let mut document = ChartDocument::new(level);
        let grid = ChartBeatGrid {
            bpm: 174.25,
            phase_media_seconds: 0.137,
            source_sha256: Some("ab".repeat(32)),
        };
        document.set_beat_grid(Some(grid.clone())).unwrap();
        assert_eq!(
            document.level, before,
            "applying a reference grid must not rewrite arbitrary LevelData"
        );
        assert_eq!(document.beat_grid, Some(grid.clone()));
        assert!(document.undo());
        assert_eq!(document.beat_grid, None);
        assert_eq!(document.level, before);
        assert!(document.redo());
        assert_eq!(document.beat_grid, Some(grid.clone()));
        assert_eq!(document.level, before);

        let serialized =
            project_level_snapshot(&document.level, document.beat_grid.as_ref(), None).unwrap();
        let reopened = read_project_chart_snapshot(&serialized).unwrap();
        assert_eq!(reopened.level_data, before);
        assert_eq!(reopened.beat_grid, Some(grid));
    }
}
