//! Portable, versioned descriptions of render inputs.
//!
//! Projects contain references and a small set of subject-specific defaults.
//! Profiles remain reusable renderer preferences, and a queued job receives a
//! resolved `RenderConfig` snapshot rather than a live project reference.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

pub const PROJECT_FORMAT: &str = "sono-renderer-project";
pub const PROJECT_SCHEMA_VERSION: u32 = 1;
pub const PROJECT_FILE_LIMIT: u64 = 16 * 1024 * 1024;
pub const RECENT_PROJECT_LIMIT: usize = 12;
const PROJECT_RECENTS_SCHEMA_VERSION: u32 = 1;

mod portable_optional_path {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::path::PathBuf;

    pub fn serialize<S>(path: &Option<PathBuf>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        path.as_ref()
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<PathBuf>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let path = Option::<String>::deserialize(deserializer)?;
        Ok(path.map(|path| PathBuf::from(path.replace('\\', "/"))))
    }
}

/// Input paths are interpreted relative to the `.sonoproject` file when they
/// are relative. Absolute paths continue to refer to external assets.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectFileInputs {
    #[serde(with = "portable_optional_path")]
    pub engine: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub level: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub resources: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub music: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub cover: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub mv: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub particle_resources: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub effects_resources: Option<PathBuf>,
    #[serde(with = "portable_optional_path")]
    pub background_resources: Option<PathBuf>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectMetadata {
    pub name: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub created_unix_time_ms: Option<u128>,
    pub updated_unix_time_ms: Option<u128>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Stable profile id is authoritative; the name is retained for display and
/// for projects created by profile stores that predate stable identifiers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ProjectProfileRef {
    pub id: Option<String>,
    pub name: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// Only project-specific render choices belong here. The rest comes from the
/// selected profile or the current renderer settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProjectOverrides {
    #[serde(with = "portable_optional_path")]
    pub output: Option<PathBuf>,
    pub start_time: Option<f64>,
    pub duration: Option<f64>,
    pub whole_chart: Option<bool>,
    pub fps: Option<u32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub layers: Option<crate::render::RenderLayers>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderProject {
    pub format: String,
    pub schema_version: u32,
    pub metadata: ProjectMetadata,
    pub inputs: ProjectFileInputs,
    pub profile: Option<ProjectProfileRef>,
    pub overrides: ProjectOverrides,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for RenderProject {
    fn default() -> Self {
        Self {
            format: PROJECT_FORMAT.into(),
            schema_version: PROJECT_SCHEMA_VERSION,
            metadata: ProjectMetadata::default(),
            inputs: ProjectFileInputs::default(),
            profile: None,
            overrides: ProjectOverrides::default(),
            extra: BTreeMap::new(),
        }
    }
}

impl RenderProject {
    pub fn from_config(
        config: &crate::render::RenderConfig,
        profile: Option<ProjectProfileRef>,
    ) -> Self {
        let now = unix_time_ms();
        Self {
            metadata: ProjectMetadata {
                created_unix_time_ms: Some(now),
                updated_unix_time_ms: Some(now),
                ..ProjectMetadata::default()
            },
            inputs: ProjectFileInputs {
                engine: nonempty_path(&config.engine),
                level: nonempty_path(&config.level),
                resources: nonempty_path(&config.resources),
                music: config.music.clone(),
                mv: config.mv.clone(),
                particle_resources: config.resource_overrides.particles.clone(),
                effects_resources: config.resource_overrides.effects.clone(),
                background_resources: config.resource_overrides.background.clone(),
                ..ProjectFileInputs::default()
            },
            profile,
            overrides: ProjectOverrides {
                output: config.output.clone(),
                ..ProjectOverrides::default()
            },
            ..Self::default()
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata =
            fs::metadata(path).with_context(|| format!("accessing project {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > PROJECT_FILE_LIMIT {
            bail!("project must be a regular file no larger than {PROJECT_FILE_LIMIT} bytes");
        }
        let bytes =
            fs::read(path).with_context(|| format!("reading project {}", path.display()))?;
        let mut project: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing project {}", path.display()))?;
        project.migrate_and_validate()?;
        Ok(project)
    }

    pub fn save(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        self.migrate_and_validate()?;
        self.metadata.updated_unix_time_ms = Some(unix_time_ms());
        let parent = parent_dir(path);
        fs::create_dir_all(parent)
            .with_context(|| format!("creating project directory {}", parent.display()))?;
        let mut stored = self.clone();
        stored.make_paths_relative(parent);
        let bytes = serde_json::to_vec_pretty(&stored).context("serializing project")?;
        if bytes.len() as u64 > PROJECT_FILE_LIMIT {
            bail!("project exceeds the {PROJECT_FILE_LIMIT}-byte size limit");
        }
        atomic_write(path, &bytes)
            .with_context(|| format!("atomically saving project {}", path.display()))
    }

    /// Resolve paths relative to the project location, then combine the
    /// project inputs with the selected profile/current config.
    pub fn resolve_config(
        &self,
        project_path: impl AsRef<Path>,
        current: &crate::render::RenderConfig,
        profiles: Option<&crate::config_profiles::ConfigProfileManager>,
        profile_override: Option<&str>,
    ) -> Result<(
        crate::render::RenderConfig,
        Option<crate::config_profiles::ConfigProfile>,
    )> {
        self.validate_schema()?;
        let profile = if let Some(selector) = profile_override {
            Some(
                profiles
                    .context("a profile was requested but the profile catalog is unavailable")?
                    .get(selector)?,
            )
        } else if let Some(reference) = &self.profile {
            if reference.id.is_some() {
                let id = reference.id.as_deref().unwrap();
                let manager = profiles.context(format!(
                    "project requires profile id {id:?}, but the profile catalog is unavailable"
                ))?;
                Some(
                    manager
                        .list()?
                        .into_iter()
                        .find(|item| item.id == id)
                        .with_context(|| {
                            format!("required project profile id {id:?} is missing")
                        })?,
                )
            } else if let Some(name) = reference.name.as_deref() {
                Some(
                    profiles
                        .context(format!("project requires profile {name:?}, but the profile catalog is unavailable"))?
                        .get(name)?,
                )
            } else {
                None
            }
        } else {
            None
        };
        let mut config = profile
            .as_ref()
            .map(|profile| profile.config.clone())
            .unwrap_or_else(|| current.clone());
        let base = project_path
            .as_ref()
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        for path in [
            self.inputs.engine.as_ref(),
            self.inputs.level.as_ref(),
            self.inputs.resources.as_ref(),
            self.inputs.music.as_ref(),
            self.inputs.cover.as_ref(),
            self.inputs.mv.as_ref(),
            self.inputs.particle_resources.as_ref(),
            self.inputs.effects_resources.as_ref(),
            self.inputs.background_resources.as_ref(),
            self.overrides.output.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if is_foreign_absolute(path) {
                bail!("project path {} is absolute for a different operating system; repair the reference", path.display());
            }
        }
        if let Some(output) = self
            .overrides
            .output
            .as_ref()
            .filter(|path| !path.is_absolute())
        {
            if relative_output_escapes(output) {
                bail!(
                    "relative project output escapes the project directory: {}",
                    output.display()
                );
            }
        }
        let resolve = |path: &Path| -> PathBuf {
            if path.is_absolute() || is_foreign_absolute(path) {
                path.to_path_buf()
            } else {
                base.join(path)
            }
        };
        config.engine = self
            .inputs
            .engine
            .as_deref()
            .map(resolve)
            .unwrap_or_default();
        config.level = self
            .inputs
            .level
            .as_deref()
            .map(resolve)
            .unwrap_or_default();
        config.resources = self
            .inputs
            .resources
            .as_deref()
            .map(resolve)
            .unwrap_or_default();
        config.music = self.inputs.music.as_deref().map(resolve);
        config.mv = self.inputs.mv.as_deref().map(resolve);
        config.resource_overrides.particles =
            self.inputs.particle_resources.as_deref().map(resolve);
        config.resource_overrides.effects = self.inputs.effects_resources.as_deref().map(resolve);
        config.resource_overrides.background =
            self.inputs.background_resources.as_deref().map(resolve);
        if let Some(path) = &self.overrides.output {
            config.output = Some(resolve(path));
        }
        if let Some(value) = self.overrides.start_time {
            config.start_time = value;
        }
        if let Some(value) = self.overrides.duration {
            config.duration = value;
        }
        if let Some(value) = self.overrides.whole_chart {
            config.whole_chart = value;
        }
        if let Some(value) = self.overrides.fps {
            config.fps = value;
        }
        if let Some(value) = self.overrides.width {
            config.width = value;
        }
        if let Some(value) = self.overrides.height {
            config.height = value;
        }
        if let Some(value) = &self.overrides.layers {
            config.layers = value.clone();
        }
        config.ui.migrate_legacy_empty_default();
        config
            .validate()
            .context("resolved project configuration is invalid")?;
        Ok((config, profile))
    }

    pub fn validate_inputs(&self, project_path: impl AsRef<Path>) -> Result<ProjectValidation> {
        self.validate_schema()?;
        let base = project_path
            .as_ref()
            .parent()
            .unwrap_or_else(|| Path::new("."));
        let resolved = self.resolved_inputs(base);
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let engine = resolved.engine.as_ref();
        let resources = resolved.resources.as_ref();
        let level = resolved.level.as_ref();
        for (role, path) in [
            ("engine", engine),
            ("level", level),
            ("resources", resources),
        ] {
            if path.is_none() {
                errors.push(format!("required {role} reference is not set"));
            }
        }
        if resolved.music.is_none() {
            warnings.push("music is not set; renders with BGM enabled will require it".into());
        }
        for (role, path) in [
            ("engine", engine),
            ("level", level),
            ("resources", resources),
            ("music", resolved.music.as_ref()),
            ("cover", resolved.cover.as_ref()),
            ("MV", resolved.mv.as_ref()),
            ("particle resources", resolved.particle_resources.as_ref()),
            ("effects resources", resolved.effects_resources.as_ref()),
            (
                "background resources",
                resolved.background_resources.as_ref(),
            ),
        ] {
            if let Some(path) = path {
                if is_foreign_absolute(path) {
                    errors.push(format!("{role} path is absolute for a different operating system and must be repaired: {}", path.display()));
                } else if !path.exists() {
                    errors.push(format!("{role} is missing: {}", path.display()));
                } else if role != "engine" && !path.is_file() {
                    errors.push(format!("{role} input must be a file: {}", path.display()));
                } else if path.is_file()
                    && fs::metadata(path).is_ok_and(|metadata| metadata.len() == 0)
                {
                    errors.push(format!("{role} input is empty: {}", path.display()));
                }
            }
        }
        let mut level_entities = None;
        let mut resource_count = None;
        if let Some(path) = engine.filter(|path| path.exists()) {
            if let Err(error) = crate::formats::load_engine(path) {
                errors.push(format!("engine could not be parsed: {error:#}"));
            }
        }
        for (role, path) in [("level", level), ("resources", resources)] {
            if let Some(path) = path.filter(|path| path.exists() && !path.is_file()) {
                errors.push(format!("{role} input must be a file: {}", path.display()));
            }
        }
        if let Some(path) = level.filter(|path| path.is_file()) {
            match crate::formats::load_level(path) {
                Ok(data) => level_entities = Some(data.entities.len()),
                Err(error) => errors.push(format!("level could not be parsed: {error:#}")),
            }
        }
        if let Some(path) = resources.filter(|path| path.is_file()) {
            match crate::formats::inspect_scp(path) {
                Ok(data) => resource_count = Some(data.resources.len()),
                Err(error) => {
                    errors.push(format!("resource package could not be parsed: {error:#}"))
                }
            }
        }
        for (role, path) in [
            ("particle resources", resolved.particle_resources.as_ref()),
            ("effects resources", resolved.effects_resources.as_ref()),
            (
                "background resources",
                resolved.background_resources.as_ref(),
            ),
        ] {
            if let Some(path) = path.filter(|path| path.is_file()) {
                if let Err(error) = crate::formats::inspect_scp(path) {
                    errors.push(format!("{role} could not be parsed: {error:#}"));
                }
            }
        }
        if let Some(path) = &resolved.music {
            if path.is_file() && !looks_like_audio(path) {
                errors.push(format!(
                    "music file signature is not a supported audio format: {}",
                    path.display()
                ));
            }
        }
        if let Some(path) = &resolved.cover {
            if path.is_file() && image::image_dimensions(path).is_err() {
                errors.push(format!(
                    "cover is not a readable supported image: {}",
                    path.display()
                ));
            }
        }
        if let Some(path) = &resolved.mv {
            if path.is_file() && !looks_like_video(path) {
                errors.push(format!(
                    "MV file signature is not a supported video format: {}",
                    path.display()
                ));
            }
        }
        let compatibility = match (engine, resources, level) {
            (Some(engine), Some(resources), Some(level))
                if engine.is_file() && resources.is_file() && level.is_file() =>
            {
                match crate::compatibility::analyze(engine, resources, level) {
                    Ok(report) => Some(report),
                    Err(error) => {
                        warnings.push(format!("compatibility analysis unavailable: {error:#}"));
                        None
                    }
                }
            }
            _ => None,
        };
        Ok(ProjectValidation {
            errors,
            warnings,
            level_entities,
            resource_count,
            compatibility,
        })
    }

    pub fn resolved_inputs(&self, base: &Path) -> ProjectFileInputs {
        let resolve = |path: &Option<PathBuf>| {
            path.as_ref().map(|path| {
                if path.is_absolute() || is_foreign_absolute(path) {
                    path.clone()
                } else {
                    base.join(path)
                }
            })
        };
        ProjectFileInputs {
            engine: resolve(&self.inputs.engine),
            level: resolve(&self.inputs.level),
            resources: resolve(&self.inputs.resources),
            music: resolve(&self.inputs.music),
            cover: resolve(&self.inputs.cover),
            mv: resolve(&self.inputs.mv),
            particle_resources: resolve(&self.inputs.particle_resources),
            effects_resources: resolve(&self.inputs.effects_resources),
            background_resources: resolve(&self.inputs.background_resources),
            extra: self.inputs.extra.clone(),
        }
    }

    fn make_paths_relative(&mut self, base: &Path) {
        for path in [
            &mut self.inputs.engine,
            &mut self.inputs.level,
            &mut self.inputs.resources,
            &mut self.inputs.music,
            &mut self.inputs.cover,
            &mut self.inputs.mv,
            &mut self.inputs.particle_resources,
            &mut self.inputs.effects_resources,
            &mut self.inputs.background_resources,
            &mut self.overrides.output,
        ]
        .into_iter()
        .flatten()
        {
            if path.is_absolute() && !is_foreign_absolute(path) {
                if let Some(relative) = relative_path(base, path) {
                    *path = relative;
                }
            }
        }
    }

    fn migrate_and_validate(&mut self) -> Result<()> {
        if self.format != PROJECT_FORMAT {
            bail!("unsupported project format {:?}", self.format);
        }
        // Schema zero was an early project draft with the same path fields and
        // no metadata/profile/override guarantees. Defaults provide its safe migration.
        if self.schema_version == 0 {
            self.schema_version = PROJECT_SCHEMA_VERSION;
        }
        self.validate_schema()
    }

    fn validate_schema(&self) -> Result<()> {
        if self.format != PROJECT_FORMAT {
            bail!("unsupported project format {:?}", self.format);
        }
        if self.schema_version != PROJECT_SCHEMA_VERSION {
            bail!(
                "unsupported project schema version {}; this renderer supports {}",
                self.schema_version,
                PROJECT_SCHEMA_VERSION
            );
        }
        for (name, value) in [
            ("name", self.metadata.name.as_deref()),
            ("author", self.metadata.author.as_deref()),
            ("description", self.metadata.description.as_deref()),
        ] {
            if value.is_some_and(|value| value.len() > 16_384 || value.contains('\0')) {
                bail!("project metadata {name} is invalid or too large");
            }
        }
        for path in [
            self.inputs.engine.as_ref(),
            self.inputs.level.as_ref(),
            self.inputs.resources.as_ref(),
            self.inputs.music.as_ref(),
            self.inputs.cover.as_ref(),
            self.inputs.mv.as_ref(),
            self.inputs.particle_resources.as_ref(),
            self.inputs.effects_resources.as_ref(),
            self.inputs.background_resources.as_ref(),
            self.overrides.output.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            if path.as_os_str().is_empty() || path.to_string_lossy().len() > 32_768 {
                bail!("project contains an empty or overlong path reference");
            }
        }
        if self
            .overrides
            .start_time
            .is_some_and(|value| !value.is_finite() || value < 0.0)
        {
            bail!("project start time must be finite and nonnegative");
        }
        if self
            .overrides
            .duration
            .is_some_and(|value| !value.is_finite() || value <= 0.0)
        {
            bail!("project duration must be finite and positive");
        }
        if self
            .overrides
            .fps
            .is_some_and(|value| !(1..=240).contains(&value))
        {
            bail!("project FPS must be in 1..=240");
        }
        if self
            .overrides
            .width
            .is_some_and(|value| value == 0 || value > 8192)
            || self
                .overrides
                .height
                .is_some_and(|value| value == 0 || value > 8192)
        {
            bail!("project resolution must be in 1..=8192");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectValidation {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub level_entities: Option<usize>,
    pub resource_count: Option<usize>,
    pub compatibility: Option<crate::compatibility::CompatibilityReport>,
}

impl ProjectValidation {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProjects {
    pub schema_version: u32,
    pub paths: Vec<PathBuf>,
}

impl Default for RecentProjects {
    fn default() -> Self {
        Self {
            schema_version: PROJECT_RECENTS_SCHEMA_VERSION,
            paths: Vec::new(),
        }
    }
}

impl RecentProjects {
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        let Ok(metadata) = fs::metadata(path) else {
            return Self::default();
        };
        if metadata.len() > 256 * 1024 {
            return Self::default();
        }
        let Ok(bytes) = fs::read(path) else {
            return Self::default();
        };
        let Ok(mut recent) = serde_json::from_slice::<Self>(&bytes) else {
            return Self::default();
        };
        if recent.schema_version != PROJECT_RECENTS_SCHEMA_VERSION {
            return Self::default();
        }
        recent.paths.retain(|candidate| candidate.is_file());
        recent.paths.truncate(RECENT_PROJECT_LIMIT);
        recent
    }

    pub fn touch(&mut self, project: &Path) {
        let path = project
            .canonicalize()
            .unwrap_or_else(|_| project.to_path_buf());
        self.paths.retain(|previous| previous != &path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_PROJECT_LIMIT);
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self)?;
        atomic_write(path.as_ref(), &bytes)
    }
}

pub fn default_recent_projects_path() -> PathBuf {
    crate::store_paths::config_root().join("recent-projects.json")
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = parent_dir(path);
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn parent_dir(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

fn nonempty_path(path: &Path) -> Option<PathBuf> {
    (!path.as_os_str().is_empty()).then(|| path.to_path_buf())
}
fn unix_time_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn relative_path(base: &Path, target: &Path) -> Option<PathBuf> {
    let target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(target)
    };
    let base = if base.is_absolute() {
        base.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(base)
    };
    let base_components: Vec<_> = base.components().collect();
    let target_components: Vec<_> = target.components().collect();
    if !same_root(
        base_components.first().copied(),
        target_components.first().copied(),
    ) {
        return None;
    }
    let mut common = 0;
    while common < base_components.len()
        && common < target_components.len()
        && same_component(base_components[common], target_components[common])
    {
        common += 1;
    }
    let mut relative = PathBuf::new();
    for component in &base_components[common..] {
        if matches!(component, Component::Normal(_)) {
            relative.push("..");
        }
    }
    for component in &target_components[common..] {
        relative.push(component.as_os_str());
    }
    Some(if relative.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        relative
    })
}

fn same_root(a: Option<Component<'_>>, b: Option<Component<'_>>) -> bool {
    match (a, b) {
        (Some(Component::Prefix(a)), Some(Component::Prefix(b))) => a
            .as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&b.as_os_str().to_string_lossy()),
        (Some(Component::RootDir), Some(Component::RootDir)) => true,
        (a, b) => a == b,
    }
}
fn same_component(a: Component<'_>, b: Component<'_>) -> bool {
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

fn relative_output_escapes(path: &Path) -> bool {
    let mut depth = 0_i32;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return true;
                }
            }
            Component::RootDir | Component::Prefix(_) => return true,
            Component::CurDir => {}
        }
    }
    false
}

#[cfg(not(windows))]
fn is_foreign_absolute(path: &Path) -> bool {
    let value = path.to_string_lossy();
    let bytes = value.as_bytes();
    bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && bytes[2] == b'/'
}

#[cfg(windows)]
fn is_foreign_absolute(path: &Path) -> bool {
    path.to_string_lossy().starts_with('/') && !path.is_absolute()
}

fn looks_like_audio(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    let Ok(read) = std::io::Read::read(&mut file, &mut header) else {
        return false;
    };
    let bytes = &header[..read];
    let is_mp4_audio = bytes.get(4..8) == Some(b"ftyp")
        && path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("m4a"));
    bytes.starts_with(b"ID3")
        || bytes.starts_with(b"OggS")
        || bytes.starts_with(b"fLaC")
        || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE")
        || bytes.starts_with(b"FORM") && bytes.get(8..12) == Some(b"AIFF")
        || is_mp4_audio
        || bytes.first() == Some(&0xff) && bytes.get(1).is_some_and(|byte| byte & 0xe0 == 0xe0)
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum InputRole {
    Engine,
    Level,
    Resources,
    ParticleResources,
    EffectsResources,
    BackgroundResources,
    Music,
    Cover,
    Mv,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InputDiscovery {
    pub candidates: BTreeMap<InputRole, Vec<PathBuf>>,
    pub project_files: Vec<PathBuf>,
    pub ignored: Vec<String>,
    pub truncated: bool,
}

impl InputDiscovery {
    /// A mapping is automatic only when the required roles each have one
    /// candidate and those candidates are distinct files.
    pub fn unique_required_mapping(&self) -> Option<BTreeMap<InputRole, PathBuf>> {
        let mut result = BTreeMap::new();
        for role in [InputRole::Engine, InputRole::Level, InputRole::Resources] {
            let values = self.candidates.get(&role)?;
            if values.len() != 1 {
                return None;
            }
            result.insert(role, values[0].clone());
        }
        let mut distinct = result.values().cloned().collect::<Vec<_>>();
        distinct.sort();
        distinct.dedup();
        (distinct.len() == result.len()).then_some(result)
    }
}

/// Inspect at most 256 files and two directory levels. Format loaders, not
/// filename stems, establish engine/level/resource candidates.
pub fn discover_inputs(dropped: &[PathBuf]) -> InputDiscovery {
    const MAX_FILES: usize = 256;
    let mut result = InputDiscovery::default();
    let mut files = Vec::new();
    for path in dropped {
        if path.is_dir() {
            collect_directory(path, 0, &mut files, &mut result);
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            result
                .ignored
                .push(format!("not accessible: {}", path.display()));
        }
        if files.len() >= MAX_FILES {
            result.truncated = true;
            break;
        }
    }
    files.sort();
    files.dedup();
    files.truncate(MAX_FILES);
    for path in files {
        if RenderProject::load(&path).is_ok() {
            result.project_files.push(path.clone());
        }
        let mut found = Vec::new();
        let is_engine = crate::formats::load_engine(&path).is_ok();
        let is_level = crate::formats::load_level(&path).is_ok();
        if is_engine {
            found.push(InputRole::Engine);
        }
        if is_level {
            found.push(InputRole::Level);
        }
        // SCP inspection accepts arbitrary ZIP containers. Engine and level
        // loaders therefore take precedence so their archives cannot also be
        // silently presented as a valid resource pack.
        if !is_engine && !is_level && crate::formats::inspect_scp(&path).is_ok() {
            found.push(InputRole::Resources);
        }
        if looks_like_audio(&path) {
            found.push(InputRole::Music);
        }
        if image::image_dimensions(&path).is_ok() {
            found.push(InputRole::Cover);
        }
        if looks_like_video(&path) {
            found.push(InputRole::Mv);
        }
        if found.is_empty() && !result.project_files.contains(&path) {
            result.ignored.push(format!(
                "no supported input format recognized: {}",
                path.display()
            ));
        }
        for role in found {
            result
                .candidates
                .entry(role)
                .or_default()
                .push(path.clone());
        }
    }
    if let Some(resource_candidates) = result
        .candidates
        .get(&InputRole::Resources)
        .filter(|candidates| candidates.len() > 1)
        .cloned()
    {
        // There is no reliable filename rule for deciding which SCP is the
        // skin, particle, effects, or background pack. Expose secondary slots
        // only when multiple valid packs make an explicit assignment useful.
        for role in [
            InputRole::ParticleResources,
            InputRole::EffectsResources,
            InputRole::BackgroundResources,
        ] {
            result.candidates.insert(role, resource_candidates.clone());
        }
    }
    result
}

fn collect_directory(
    path: &Path,
    depth: usize,
    files: &mut Vec<PathBuf>,
    result: &mut InputDiscovery,
) {
    const MAX_FILES: usize = 256;
    const MAX_DEPTH: usize = 2;
    if depth > MAX_DEPTH || files.len() >= MAX_FILES {
        result.truncated = true;
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        result
            .ignored
            .push(format!("directory could not be read: {}", path.display()));
        return;
    };
    for entry in entries
        .flatten()
        .take(MAX_FILES.saturating_sub(files.len()))
    {
        let child = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_file() {
            files.push(child);
        } else if meta.is_dir() && depth < MAX_DEPTH {
            collect_directory(&child, depth + 1, files, result);
        }
        if files.len() >= MAX_FILES {
            result.truncated = true;
            break;
        }
    }
}

fn looks_like_video(path: &Path) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 16];
    let Ok(read) = std::io::Read::read(&mut file, &mut header) else {
        return false;
    };
    let bytes = &header[..read];
    bytes.get(4..8) == Some(b"ftyp")
        || bytes.starts_with(b"\x1aE\xdf\xa3")
        || bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"AVI ")
}

/// Legacy one-shot validator retained for the `validate-project` CLI command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectInputs {
    pub engine_package: PathBuf,
    pub resource_package: PathBuf,
    pub level_data: PathBuf,
    pub music: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_mv: Option<PathBuf>,
}

#[derive(Debug)]
pub struct ValidatedProject {
    pub engine: crate::formats::EnginePackage,
    pub resources: crate::formats::ScpIndex,
    pub level: crate::formats::LevelData,
    pub music: PathBuf,
    pub cover: Option<PathBuf>,
    pub custom_mv: Option<PathBuf>,
}

impl ProjectInputs {
    pub fn validate(&self) -> Result<ValidatedProject> {
        require_file(&self.engine_package, "engine ZIP")?;
        if !self
            .engine_package
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("zip"))
        {
            bail!(
                "engine input must be an engine ZIP: {}",
                self.engine_package.display()
            );
        }
        require_file(&self.resource_package, "SCP/resource package")?;
        require_file(&self.level_data, "level data")?;
        require_file(&self.music, "music/audio")?;
        if let Some(cover) = &self.cover {
            require_file(cover, "cover image")?;
        }
        if let Some(mv) = &self.custom_mv {
            require_file(mv, "custom MV/video")?;
        }
        let engine = crate::formats::load_engine(&self.engine_package)
            .with_context(|| format!("loading engine {}", self.engine_package.display()))?;
        let resources = crate::formats::inspect_scp(&self.resource_package)
            .with_context(|| format!("loading resources {}", self.resource_package.display()))?;
        let level = crate::formats::load_level(&self.level_data)
            .with_context(|| format!("loading level {}", self.level_data.display()))?;
        Ok(ValidatedProject {
            engine,
            resources,
            level,
            music: self.music.clone(),
            cover: self.cover.clone(),
            custom_mv: self.custom_mv.clone(),
        })
    }
}

fn require_file(path: &Path, label: &str) -> Result<()> {
    let metadata =
        fs::metadata(path).with_context(|| format!("accessing {label} at {}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        bail!("{label} must be a non-empty file: {}", path.display());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_round_trip_keeps_unknown_fields_and_relative_paths() {
        let directory = tempfile::tempdir().unwrap();
        let assets = directory.path().join("assets");
        fs::create_dir(&assets).unwrap();
        let engine = assets.join("engine.zip");
        fs::write(&engine, b"placeholder").unwrap();
        let project_path = directory.path().join("chart.sonoproject");
        let mut project = RenderProject::default();
        project.inputs.engine = Some(engine.clone());
        project.profile = Some(ProjectProfileRef {
            id: Some("stable-profile-id".into()),
            name: Some("Stable profile".into()),
            extra: BTreeMap::from([("future_profile_hint".into(), Value::Bool(true))]),
        });
        project
            .extra
            .insert("future_field".into(), Value::String("preserved".into()));
        project.save(&project_path).unwrap();
        let loaded = RenderProject::load(&project_path).unwrap();
        assert_eq!(loaded.extra["future_field"], "preserved");
        assert_eq!(
            loaded.profile.as_ref().unwrap().extra["future_profile_hint"],
            true
        );
        assert_eq!(
            loaded.inputs.engine.as_deref(),
            Some(Path::new("assets/engine.zip"))
        );
        assert_eq!(
            loaded.resolved_inputs(directory.path()).engine.unwrap(),
            engine
        );
    }

    #[test]
    fn project_paths_serialize_with_portable_forward_slashes() {
        let mut project = RenderProject::default();
        project.inputs.level = Some(PathBuf::from("assets\\charts\\chart.json.gz"));
        let json = serde_json::to_string(&project).unwrap();
        assert!(json.contains("assets/charts/chart.json.gz"));
        let restored: RenderProject = serde_json::from_str(&json).unwrap();
        assert!(restored
            .inputs
            .level
            .unwrap()
            .to_string_lossy()
            .contains("assets"));
    }

    #[test]
    fn schema_zero_migrates_and_keeps_unknown_fields() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.sonoproject");
        fs::write(&path, br#"{"format":"sono-renderer-project","schema_version":0,"old_extension":{"keep":true}}"#).unwrap();
        let migrated = RenderProject::load(&path).unwrap();
        assert_eq!(migrated.schema_version, PROJECT_SCHEMA_VERSION);
        assert_eq!(migrated.extra["old_extension"]["keep"], true);
    }

    #[test]
    fn foreign_absolute_paths_are_not_resolved_as_project_relative_paths() {
        #[cfg(not(windows))]
        {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("foreign.sonoproject");
            let mut project = RenderProject::default();
            project.inputs.engine = Some(PathBuf::from("C:/renderer/engine.zip"));
            project.save(&path).unwrap();
            let loaded = RenderProject::load(&path).unwrap();
            assert!(loaded
                .resolve_config(&path, &crate::render::RenderConfig::default(), None, None)
                .is_err());
            let report = loaded.validate_inputs(&path).unwrap();
            assert!(report
                .errors
                .iter()
                .any(|error| error.contains("different operating system")));
        }
    }

    #[test]
    fn unknown_future_schema_is_rejected_without_overwriting_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future.sonoproject");
        fs::write(
            &path,
            br#"{"format":"sono-renderer-project","schema_version":88}"#,
        )
        .unwrap();
        assert!(RenderProject::load(&path)
            .unwrap_err()
            .to_string()
            .contains("schema version"));
        assert!(fs::read_to_string(path).unwrap().contains("88"));
    }

    #[test]
    fn atomic_save_replaces_previous_valid_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("replace.sonoproject");
        let mut project = RenderProject::default();
        project.metadata.name = Some("old".into());
        project.save(&path).unwrap();
        project.metadata.name = Some("new".into());
        project.save(&path).unwrap();
        assert_eq!(
            RenderProject::load(path).unwrap().metadata.name.as_deref(),
            Some("new")
        );
    }

    #[test]
    fn relative_paths_follow_moved_project_directory() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let moved = directory.path().join("moved");
        fs::create_dir_all(first.join("media")).unwrap();
        fs::create_dir_all(moved.join("media")).unwrap();
        fs::write(first.join("media/song.ogg"), b"OggSfixture").unwrap();
        fs::copy(first.join("media/song.ogg"), moved.join("media/song.ogg")).unwrap();
        let mut project = RenderProject::default();
        project.inputs.music = Some(first.join("media/song.ogg"));
        project.make_paths_relative(&first);
        assert_eq!(
            project.resolved_inputs(&moved).music.unwrap(),
            moved.join("media/song.ogg")
        );
        assert!(project.resolved_inputs(&moved).music.unwrap().is_file());
    }

    #[test]
    fn project_rejects_nonfinite_or_out_of_range_overrides() {
        let mut project = RenderProject::default();
        project.overrides.start_time = Some(f64::NAN);
        assert!(project.validate_schema().is_err());
        project.overrides.start_time = None;
        project.overrides.width = Some(9000);
        assert!(project.validate_schema().is_err());
    }

    #[test]
    fn audio_detection_uses_file_signature_not_just_filename() {
        let directory = tempfile::tempdir().unwrap();
        let fake = directory.path().join("music.mp3");
        fs::write(&fake, b"not audio").unwrap();
        assert!(!looks_like_audio(&fake));
        fs::write(&fake, b"ID3\x04\x00\x00\x00\x00\x00\x00").unwrap();
        assert!(looks_like_audio(&fake));
    }

    #[test]
    fn required_profile_resolves_by_stable_id_after_rename_and_missing_id_fails() {
        let directory = tempfile::tempdir().unwrap();
        let manager = crate::config_profiles::ConfigProfileManager::open_at(
            directory.path().join("profiles.json"),
        )
        .unwrap();
        let mut profile_config = crate::render::RenderConfig::default();
        profile_config.width = 1280;
        let profile = manager.create("Before rename", profile_config).unwrap();
        let project_path = directory.path().join("chart.sonoproject");
        let mut project = RenderProject::default();
        project.profile = Some(ProjectProfileRef {
            id: Some(profile.id.clone()),
            name: Some(profile.name.clone()),
            ..ProjectProfileRef::default()
        });
        manager.rename(&profile.id, "After rename").unwrap();
        let (resolved, resolved_profile) = project
            .resolve_config(
                &project_path,
                &crate::render::RenderConfig::default(),
                Some(&manager),
                None,
            )
            .unwrap();
        assert_eq!(resolved.width, 1280);
        assert_eq!(resolved_profile.unwrap().name, "After rename");
        manager.delete(&profile.id).unwrap();
        assert!(project
            .resolve_config(
                &project_path,
                &crate::render::RenderConfig::default(),
                Some(&manager),
                None,
            )
            .unwrap_err()
            .to_string()
            .contains("missing"));
    }

    #[test]
    fn queued_job_keeps_resolved_project_config_after_project_changes() {
        let directory = tempfile::tempdir().unwrap();
        let assets = directory.path().join("assets");
        fs::create_dir_all(&assets).unwrap();
        let engine = assets.join("engine.zip");
        let resources = assets.join("skin.scp");
        let level = assets.join("chart.json");
        let music = assets.join("song.wav");
        for path in [&engine, &resources, &level] {
            fs::write(path, b"fixture").unwrap();
        }
        fs::write(&music, b"RIFF\x00\x00\x00\x00WAVEfmt ").unwrap();
        let project_path = directory.path().join("chart.sonoproject");
        let mut base = crate::render::RenderConfig::default();
        base.width = 1280;
        let profiles = crate::config_profiles::ConfigProfileManager::open_at(
            directory.path().join("profiles.json"),
        )
        .unwrap();
        let profile = profiles.create("Render profile", base).unwrap();
        let mut project = RenderProject::default();
        project.inputs.engine = Some(engine.clone());
        project.inputs.resources = Some(resources.clone());
        project.inputs.level = Some(level.clone());
        project.inputs.music = Some(music.clone());
        project.overrides.output = Some(directory.path().join("project-output.mp4"));
        project.profile = Some(ProjectProfileRef {
            id: Some(profile.id.clone()),
            name: Some(profile.name.clone()),
            ..ProjectProfileRef::default()
        });
        project.save(&project_path).unwrap();
        let project = RenderProject::load(&project_path).unwrap();
        let (mut config, source) = project
            .resolve_config(
                &project_path,
                &crate::render::RenderConfig::default(),
                Some(&profiles),
                None,
            )
            .unwrap();
        config.layers.bgm = false;
        let manager = crate::job_manager::RenderJobManager::open_at(
            directory.path().join("jobs.json"),
            config.job_manager.clone(),
        )
        .unwrap();
        let id = manager
            .enqueue_from_profile(
                config.clone(),
                None,
                source.unwrap().id,
                "renamed later".into(),
            )
            .unwrap();
        let mut edited = project.clone();
        edited.inputs.engine = Some(directory.path().join("new-engine.zip"));
        edited.save(&project_path).unwrap();
        let queued = manager.inspect(&id).unwrap();
        assert_eq!(queued.config.engine, engine);
        assert_eq!(queued.config.width, 1280);
        assert_eq!(queued.profile_name.as_deref(), Some("renamed later"));
    }

    #[test]
    fn ambiguous_role_mapping_is_never_auto_selected() {
        let mut discovery = InputDiscovery::default();
        discovery.candidates.insert(
            InputRole::Engine,
            vec!["engine-a.zip".into(), "engine-b.zip".into()],
        );
        discovery
            .candidates
            .insert(InputRole::Level, vec!["chart.json.gz".into()]);
        discovery
            .candidates
            .insert(InputRole::Resources, vec!["skin.scp".into()]);
        assert!(discovery.unique_required_mapping().is_none());
        discovery
            .candidates
            .insert(InputRole::Engine, vec!["same.zip".into()]);
        discovery
            .candidates
            .insert(InputRole::Level, vec!["same.zip".into()]);
        assert!(discovery.unique_required_mapping().is_none());
    }

    #[test]
    fn actual_next_sekai_fixture_passes_project_preflight_when_present() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = root.join("TestingSuite/Next Sekai Engine/engine/Next Sekai Engine.zip");
        let resources = root.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let level = root.join("TestingSuite/Next Sekai Engine/levels/Yukopi - Shouji Ichiban/Yukopi - Shouji Ichiban.json.gz");
        let music =
            root.join("TestingSuite/Next Sekai Engine/levels/Yukopi - Shouji Ichiban/audio.mp3");
        if ![&engine, &resources, &level, &music]
            .iter()
            .all(|path| path.is_file())
        {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let discovery = discover_inputs(&[
            engine.clone(),
            resources.clone(),
            level.clone(),
            music.clone(),
        ]);
        let mapping = discovery
            .unique_required_mapping()
            .unwrap_or_else(|| panic!("required candidate mapping was not unique: {discovery:#?}"));
        assert_eq!(mapping.get(&InputRole::Engine), Some(&engine));
        assert_eq!(mapping.get(&InputRole::Level), Some(&level));
        assert_eq!(mapping.get(&InputRole::Resources), Some(&resources));
        assert_eq!(
            discovery.candidates.get(&InputRole::Music).unwrap(),
            &[music.clone()]
        );
        let project_path = directory.path().join("fixture.sonoproject");
        let mut project = RenderProject::default();
        project.inputs.engine = Some(engine);
        project.inputs.resources = Some(resources);
        project.inputs.level = Some(level);
        project.inputs.music = Some(music);
        project.save(&project_path).unwrap();
        let report = RenderProject::load(&project_path)
            .unwrap()
            .validate_inputs(&project_path)
            .unwrap();
        assert!(report.is_valid(), "{:#?}", report.errors);
        assert!(report.level_entities.unwrap_or_default() > 0);
        assert!(report.resource_count.unwrap_or_default() > 0);
        assert!(report.compatibility.is_some());
    }

    #[test]
    #[ignore = "runs a real fixture render through the persistent queue and FFmpeg"]
    fn saved_project_can_queue_and_render_a_real_fixture() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let engine = root.join("TestingSuite/Next Sekai Engine/engine/Next Sekai Engine.zip");
        let resources = root.join("TestingSuite/Next Sekai Engine/skin/ProSeka Faithful 0.8.3.scp");
        let level = root.join("TestingSuite/Next Sekai Engine/levels/Yukopi - Shouji Ichiban/Yukopi - Shouji Ichiban.json.gz");
        let music =
            root.join("TestingSuite/Next Sekai Engine/levels/Yukopi - Shouji Ichiban/audio.mp3");
        if ![&engine, &resources, &level, &music]
            .iter()
            .all(|path| path.is_file())
        {
            return;
        }
        let directory = tempfile::tempdir().unwrap();
        let project_path = directory.path().join("fixture.sonoproject");
        let output = directory.path().join("fixture.mp4");
        let mut project = RenderProject::default();
        project.inputs.engine = Some(engine);
        project.inputs.resources = Some(resources);
        project.inputs.level = Some(level);
        project.inputs.music = Some(music);
        project.overrides.output = Some(output.clone());
        project.overrides.duration = Some(0.5);
        project.overrides.fps = Some(2);
        project.overrides.width = Some(128);
        project.overrides.height = Some(72);
        project.overrides.layers = Some(crate::render::RenderLayers {
            particles: false,
            sfx: false,
            bgm: true,
        });
        project.save(&project_path).unwrap();
        let reopened = RenderProject::load(&project_path).unwrap();
        let (mut config, _) = reopened
            .resolve_config(
                &project_path,
                &crate::render::RenderConfig::default(),
                None,
                None,
            )
            .unwrap();
        config.backend = crate::render::RenderBackend::Cpu;
        config.execution_mode = crate::sono_gcc::WatchExecutionMode::Interpreter;
        config.compile_sono_gcc_on_demand = false;
        let queue = crate::job_manager::RenderJobManager::open_at(
            directory.path().join("jobs.json"),
            config.job_manager.clone(),
        )
        .unwrap();
        let id = queue.enqueue(config, None).unwrap();
        queue.start().unwrap();
        queue.wait_until_idle().unwrap();
        let job = queue.inspect(&id).unwrap();
        assert_eq!(
            job.state,
            crate::job_manager::JobState::Completed,
            "{:#?}",
            job.diagnostic
        );
        assert!(output.is_file());
        assert!(fs::metadata(output).unwrap().len() > 0);
    }

    #[test]
    fn relative_output_may_not_escape_project_directory() {
        let mut project = RenderProject::default();
        project.overrides.output = Some(PathBuf::from("../../outside.mp4"));
        let result = project.resolve_config(
            "project.sonoproject",
            &crate::render::RenderConfig::default(),
            None,
            None,
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("escapes the project directory"));
    }
}
