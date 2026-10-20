//! Portable records of resolved render inputs and the configuration that ran.
//!
//! A manifest is exported from immutable render history. It is deliberately
//! not a project (editable intent), queue entry (active lifecycle), or cache
//! archive (generated acceleration data).

use crate::{
    content_hash::{sha256_file, sha256_hex},
    render::RenderConfig,
    render_history::RenderHistoryRecord,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub const MANIFEST_FORMAT: &str = "sono-render-manifest";
pub const MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MAX_MANIFEST_BYTES: u64 = 16 * 1024 * 1024;
const MAX_INPUTS: usize = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RenderManifest {
    pub format: String,
    pub schema_version: u32,
    pub manifest_sha256: String,
    pub renderer: ManifestRenderer,
    pub execution: ManifestExecution,
    pub provenance: ManifestProvenance,
    pub inputs: Vec<ManifestInput>,
    /// Redacted, resolved config snapshot. File paths inside it are discovery
    /// hints only; `inputs` content identities determine whether they match.
    pub effective_config: RenderConfig,
    pub output: ManifestOutput,
    pub references: ManifestReferences,
    /// Unknown top-level fields are retained for forward-compatible inspection.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Default for RenderManifest {
    fn default() -> Self {
        Self {
            format: MANIFEST_FORMAT.into(),
            schema_version: MANIFEST_SCHEMA_VERSION,
            manifest_sha256: String::new(),
            renderer: ManifestRenderer::default(),
            execution: ManifestExecution::default(),
            provenance: ManifestProvenance::default(),
            inputs: Vec::new(),
            effective_config: RenderConfig::default(),
            output: ManifestOutput::default(),
            references: ManifestReferences::default(),
            extra: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ManifestRenderer {
    pub version: String,
    pub commit: Option<String>,
    /// Declared target semantics, not a claim that every operation has been
    /// independently verified against the official client.
    pub runtime_semantics: String,
}

impl Default for ManifestRenderer {
    fn default() -> Self {
        Self {
            version: String::new(),
            commit: None,
            runtime_semantics: "sonolus-v1.1.4-target".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ManifestExecution {
    pub requested_backend: String,
    pub effective_backend: String,
    pub target_os: String,
    pub target_architecture: String,
    pub gcc_codegen_version: Option<String>,
    pub gcc_runtime_abi: Option<String>,
    pub gcc_cache_state: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ManifestProvenance {
    pub created_unix_time_ms: u128,
    pub history_id: Option<String>,
    pub job_id: Option<String>,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    /// Only a file-name hint is retained for the originating project.
    pub project_name_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestInput {
    /// Stable config-slot identity used by `--asset ID=PATH` and the GUI repair.
    pub id: String,
    pub role: String,
    pub active: bool,
    pub path_hint: Option<PathBuf>,
    pub sha256: Option<String>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManifestOutput {
    pub path_hint: Option<PathBuf>,
    pub exists: bool,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ManifestReferences {
    /// Existing renderer references, retained with their original algorithms.
    pub rgb_sequence_sha1: Option<String>,
    pub sfx_event_sha1: Option<String>,
    pub sfx_pcm_sha1: Option<String>,
    pub raw_rgb_sequence_available: bool,
    pub audio_pcm_hash_available: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    Match,
    Missing,
    Mismatch,
    NotApplicable,
    Unknown,
    Different,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DifferenceCategory {
    InputContent,
    SemanticConfiguration,
    Presentation,
    OutputEncoding,
    BackendEnvironment,
    Provenance,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VerificationItem {
    pub component: String,
    pub category: DifferenceCategory,
    pub status: VerificationStatus,
    pub expected: Option<String>,
    pub actual: Option<String>,
    pub affects_semantics: bool,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReproducibilityReport {
    pub manifest_sha256: String,
    pub schema_supported: bool,
    pub can_reproduce_exact_backend: bool,
    pub can_reproduce_semantic_inputs: bool,
    pub strongest_stored_reference: String,
    pub items: Vec<VerificationItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestDifference {
    pub category: DifferenceCategory,
    pub component: String,
    pub left: Option<String>,
    pub right: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManifestComparison {
    pub left_sha256: String,
    pub right_sha256: String,
    pub input_content_matches: bool,
    pub semantic_config_matches: bool,
    pub raw_frame_reference_matches: Option<bool>,
    pub audio_reference_matches: Option<bool>,
    pub differences: Vec<ManifestDifference>,
}

#[derive(Debug, Clone)]
struct InputSpec {
    id: String,
    role: String,
    active: bool,
    path: Option<PathBuf>,
}

impl RenderManifest {
    pub fn from_history(record: &RenderHistoryRecord) -> Result<Self> {
        Self::from_history_with_performance(record, None)
    }

    /// Add build/compiler identity from the authoritative execution telemetry
    /// associated with this history attempt. The values are never sampled
    /// from the process exporting an older history record.
    pub fn from_history_with_performance(
        record: &RenderHistoryRecord,
        performance: Option<&crate::performance::RenderPerformanceRecord>,
    ) -> Result<Self> {
        record.validate()?;
        if let Some(performance) = performance {
            if record.performance_record_id.as_deref() != Some(performance.id.as_str()) {
                bail!("performance record does not belong to the selected history attempt");
            }
        }
        let mut effective_config =
            crate::render_history::sanitize_config_snapshot(&record.config_snapshot);
        // Notifications and scheduling are installation concerns. The
        // effective render choices are retained, but credentials/references
        // and unrelated queue settings are deliberately absent.
        effective_config.notifications = crate::notifications::NotificationConfig {
            enabled: false,
            ..Default::default()
        };
        effective_config.job_manager = crate::job_config::JobManagerConfig::default();

        let mut inputs = Vec::new();
        for spec in input_specs(&effective_config) {
            if inputs.len() >= MAX_INPUTS {
                bail!("render config references more than {MAX_INPUTS} assets");
            }
            let (sha256, size_bytes) = if spec.active {
                match spec.path.as_deref() {
                    Some(path) if fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) => {
                        let (hash, size) = stable_file_identity(path)?;
                        (Some(hash), Some(size))
                    }
                    _ => (None, None),
                }
            } else {
                (None, None)
            };
            inputs.push(ManifestInput {
                id: spec.id,
                role: spec.role,
                active: spec.active,
                path_hint: spec.path,
                sha256,
                size_bytes,
            });
        }

        let mut manifest = Self {
            format: MANIFEST_FORMAT.into(),
            schema_version: MANIFEST_SCHEMA_VERSION,
            manifest_sha256: String::new(),
            renderer: ManifestRenderer {
                version: record.renderer_version.clone(),
                commit: performance.and_then(|record| record.environment.renderer_commit.clone()),
                runtime_semantics: "sonolus-v1.1.4-target".into(),
            },
            execution: ManifestExecution {
                requested_backend: match effective_config.execution_mode {
                    crate::sono_gcc::WatchExecutionMode::Interpreter => "sono_vm",
                    crate::sono_gcc::WatchExecutionMode::SonoGcc => "sono_gcc",
                }
                .into(),
                effective_backend: record
                    .effective_backend_status
                    .clone()
                    .unwrap_or_else(|| record.execution_backend.clone()),
                target_os: record.operating_system.clone(),
                target_architecture: record.architecture.clone(),
                gcc_codegen_version: performance
                    .and_then(|record| record.gcc_codegen_version.clone()),
                gcc_runtime_abi: performance.and_then(|record| record.gcc_runtime_abi.clone()),
                gcc_cache_state: record.cache_state.clone(),
            },
            provenance: ManifestProvenance {
                created_unix_time_ms: now_unix_ms(),
                history_id: Some(record.id.clone()),
                job_id: Some(record.job_id.clone()),
                profile_id: record.profile_id.clone(),
                profile_name: record.profile_name.clone(),
                project_name_hint: record
                    .project_path
                    .as_ref()
                    .and_then(|path| path.file_name())
                    .map(|name| name.to_string_lossy().into_owned()),
            },
            inputs,
            effective_config,
            output: ManifestOutput {
                path_hint: record.output.path.clone(),
                exists: record.output.exists,
                size_bytes: record.output.size_bytes,
            },
            references: ManifestReferences {
                rgb_sequence_sha1: record.rgb_sequence_sha1.clone(),
                sfx_event_sha1: record.sfx_event_sha1.clone(),
                sfx_pcm_sha1: record.sfx_pcm_sha1.clone(),
                raw_rgb_sequence_available: record.rgb_sequence_sha1.is_some(),
                audio_pcm_hash_available: record.sfx_pcm_sha1.is_some(),
            },
            extra: BTreeMap::new(),
        };
        manifest.seal()?;
        Ok(manifest)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let metadata =
            fs::metadata(path).with_context(|| format!("reading manifest {}", path.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
            bail!("manifest must be a regular file no larger than {MAX_MANIFEST_BYTES} bytes");
        }
        let bytes =
            fs::read(path).with_context(|| format!("reading manifest {}", path.display()))?;
        let header: Value = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        let format = header
            .get("format")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if format != MANIFEST_FORMAT {
            bail!("unsupported render manifest format {format:?}");
        }
        let version = header
            .get("schema_version")
            .and_then(Value::as_u64)
            .context("render manifest has no valid schema_version")?;
        if version != u64::from(MANIFEST_SCHEMA_VERSION) {
            bail!("render manifest schema {version} is unsupported; this renderer supports {MANIFEST_SCHEMA_VERSION}");
        }
        let mut manifest: Self = serde_json::from_slice(&bytes)
            .with_context(|| format!("decoding manifest {}", path.display()))?;
        let base = path.parent().unwrap_or_else(|| Path::new("."));
        let foreign_hints = manifest
            .inputs
            .iter()
            .filter_map(|input| {
                input
                    .path_hint
                    .as_ref()
                    .filter(|hint| is_foreign_absolute_path(hint))
                    .map(|hint| (input.id.clone(), hint.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        manifest.effective_config.resolve_config_paths(base);
        apply_overrides(
            &mut manifest.effective_config,
            &manifest.inputs,
            &foreign_hints,
        )?;
        for input in &mut manifest.inputs {
            if let Some(hint) = &mut input.path_hint {
                if !hint.is_absolute() && !is_foreign_absolute_path(hint) {
                    *hint = base.join(&*hint);
                }
            }
        }
        if let Some(hint) = &mut manifest.output.path_hint {
            if !hint.is_absolute() && !is_foreign_absolute_path(hint) {
                *hint = base.join(&*hint);
            } else if is_foreign_absolute_path(hint) {
                manifest.effective_config.output = Some(hint.clone());
            }
        }
        sanitize_manifest_config(&mut manifest.effective_config);
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn save_atomic(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if path.exists() {
            bail!("refusing to replace existing manifest {}", path.display());
        }
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)
            .with_context(|| format!("creating manifest directory {}", parent.display()))?;
        let mut stored = self.clone();
        sanitize_manifest_config(&mut stored.effective_config);
        stored.make_paths_relative(parent);
        stored.seal()?;
        let bytes = serde_json::to_vec_pretty(&stored).context("serializing render manifest")?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            bail!("manifest exceeds the {MAX_MANIFEST_BYTES}-byte size limit");
        }
        let mut temp = tempfile::Builder::new()
            .prefix(".render-manifest-")
            .suffix(".tmp")
            .tempfile_in(parent)
            .context("creating atomic manifest temporary file")?;
        temp.write_all(&bytes)?;
        temp.as_file().sync_all()?;
        match temp.persist_noclobber(path) {
            Ok(_) => Ok(()),
            Err(_error) if path.exists() => {
                bail!("refusing to replace existing manifest {}", path.display())
            }
            Err(error) => {
                Err(error.error).with_context(|| format!("publishing manifest {}", path.display()))
            }
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != MANIFEST_FORMAT {
            bail!("unsupported render manifest format {:?}", self.format);
        }
        if self.schema_version != MANIFEST_SCHEMA_VERSION {
            bail!("unsupported render manifest schema {}", self.schema_version);
        }
        if self.inputs.len() > MAX_INPUTS {
            bail!("manifest has too many input references");
        }
        let mut ids = HashSet::new();
        for input in &self.inputs {
            if input.id.is_empty() || input.id.len() > 512 || !ids.insert(input.id.as_str()) {
                bail!("manifest has an empty, overlong, or duplicate asset ID");
            }
            if let Some(hash) = &input.sha256 {
                validate_sha256(hash)?;
            }
            if input.active && input.sha256.is_some() != input.size_bytes.is_some() {
                bail!(
                    "active manifest input {} has incomplete identity metadata",
                    input.id
                );
            }
        }
        if self.manifest_sha256 != self.canonical_sha256()? {
            bail!("manifest canonical SHA-256 does not match its contents");
        }
        Ok(())
    }

    pub fn canonical_sha256(&self) -> Result<String> {
        let mut config = serde_json::to_value(&self.effective_config)?;
        normalize_config_asset_paths(&mut config, &self.inputs)?;
        if let Some(object) = config.as_object_mut() {
            // Output location, notification delivery, and scheduling policy
            // do not affect the rendered pixels/audio.
            object.insert("output".into(), Value::Null);
            object.remove("notifications");
            object.remove("job_manager");
        }
        let identity = json!({
            "format": self.format,
            "schema_version": self.schema_version,
            "renderer": self.renderer,
            "execution": {
                "requested_backend": self.execution.requested_backend,
                "effective_backend": self.execution.effective_backend,
                "target_os": self.execution.target_os,
                "target_architecture": self.execution.target_architecture,
                "gcc_codegen_version": self.execution.gcc_codegen_version,
                "gcc_runtime_abi": self.execution.gcc_runtime_abi,
            },
            "inputs": self.inputs.iter().map(|input| json!({
                "id": input.id,
                "role": input.role,
                "active": input.active,
                "sha256": input.sha256,
                "size_bytes": input.size_bytes,
            })).collect::<Vec<_>>(),
            "effective_config": config,
            "output": {"exists": self.output.exists, "size_bytes": self.output.size_bytes},
            "references": self.references,
            "unknown_fields": self.extra,
        });
        Ok(sha256_hex(&serde_json::to_vec(&canonical_value(identity))?))
    }

    fn seal(&mut self) -> Result<()> {
        self.manifest_sha256 = self.canonical_sha256()?;
        Ok(())
    }

    fn make_paths_relative(&mut self, base: &Path) {
        self.effective_config.relativize_config_paths(base);
        for input in &mut self.inputs {
            if let Some(path) = &mut input.path_hint {
                crate::render::relativize_path(path, base);
            }
        }
        if let Some(path) = &mut self.output.path_hint {
            crate::render::relativize_path(path, base);
        }
    }

    pub fn verify(&self, overrides: &BTreeMap<String, PathBuf>) -> Result<ReproducibilityReport> {
        self.validate()?;
        let mut items = Vec::new();
        let mut all_inputs_match = true;
        for input in &self.inputs {
            if !input.active {
                items.push(VerificationItem {
                    component: input.role.clone(),
                    category: DifferenceCategory::InputContent,
                    status: VerificationStatus::NotApplicable,
                    expected: input.sha256.clone(),
                    actual: None,
                    affects_semantics: false,
                    explanation: "This configured asset is inactive for the recorded render."
                        .into(),
                });
                continue;
            }
            let candidate = overrides.get(&input.id).or(input.path_hint.as_ref());
            let item = match candidate {
                None => {
                    all_inputs_match = false;
                    verify_item(
                        input,
                        None,
                        None,
                        VerificationStatus::Missing,
                        "No path hint or replacement was supplied.",
                    )
                }
                Some(path) if !path.is_file() => {
                    all_inputs_match = false;
                    verify_item(
                        input,
                        Some(path),
                        None,
                        VerificationStatus::Missing,
                        "The hinted or selected file is unavailable.",
                    )
                }
                Some(path) => {
                    let (hash, _) = stable_file_identity(path)?;
                    if input.sha256.is_none() {
                        all_inputs_match = false;
                        verify_item(
                            input,
                            Some(path),
                            Some(hash),
                            VerificationStatus::Unknown,
                            "The manifest did not capture an identity for this active input.",
                        )
                    } else if input.sha256.as_deref() == Some(hash.as_str()) {
                        verify_item(
                            input,
                            Some(path),
                            Some(hash),
                            VerificationStatus::Match,
                            "SHA-256 content identity matches.",
                        )
                    } else {
                        all_inputs_match = false;
                        verify_item(
                            input,
                            Some(path),
                            Some(hash),
                            VerificationStatus::Mismatch,
                            "The selected file content does not match the recorded SHA-256.",
                        )
                    }
                }
            };
            items.push(item);
        }

        let current_version = env!("CARGO_PKG_VERSION");
        let version_match = self.renderer.version == current_version;
        items.push(VerificationItem {
            component: "Sono-Renderer version".into(),
            category: DifferenceCategory::BackendEnvironment,
            status: if version_match {
                VerificationStatus::Match
            } else {
                VerificationStatus::Different
            },
            expected: Some(self.renderer.version.clone()),
            actual: Some(current_version.into()),
            affects_semantics: true,
            explanation: if version_match {
                "Renderer package version matches."
            } else {
                "Renderer version differs; semantic behavior may differ."
            }
            .into(),
        });
        let current_commit = option_env!("SONO_RENDERER_GIT_COMMIT").map(str::to_owned);
        let commit_matches = if let Some(expected_commit) = &self.renderer.commit {
            let matches = current_commit.as_ref() == Some(expected_commit);
            items.push(VerificationItem {
                component: "Sono-Renderer build commit".into(),
                category: DifferenceCategory::BackendEnvironment,
                status: if matches {
                    VerificationStatus::Match
                } else if current_commit.is_some() {
                    VerificationStatus::Different
                } else {
                    VerificationStatus::Unknown
                },
                expected: Some(expected_commit.clone()),
                actual: current_commit.clone(),
                affects_semantics: true,
                explanation: if matches {
                    "Build commit matches the recorded renderer.".into()
                } else {
                    "The recorded build commit is unavailable or differs; exact renderer reproduction cannot be established.".into()
                },
            });
            matches
        } else {
            true
        };
        let semantic_match = self.renderer.runtime_semantics == "sonolus-v1.1.4-target";
        items.push(VerificationItem {
            component: "Runtime semantic target".into(),
            category: DifferenceCategory::SemanticConfiguration,
            status: if semantic_match { VerificationStatus::Match } else { VerificationStatus::Different },
            expected: Some(self.renderer.runtime_semantics.clone()),
            actual: Some("sonolus-v1.1.4-target".into()),
            affects_semantics: true,
            explanation: "This is the renderer's declared Watch target; it does not claim every operation is independently client-verified.".into(),
        });
        let same_os = self.execution.target_os == std::env::consts::OS;
        let same_arch = self.execution.target_architecture == std::env::consts::ARCH;
        for (name, expected, actual, matches) in [
            (
                "Target operating system",
                self.execution.target_os.as_str(),
                std::env::consts::OS,
                same_os,
            ),
            (
                "Target architecture",
                self.execution.target_architecture.as_str(),
                std::env::consts::ARCH,
                same_arch,
            ),
        ] {
            items.push(VerificationItem {
                component: name.into(),
                category: DifferenceCategory::BackendEnvironment,
                status: if matches { VerificationStatus::Match } else { VerificationStatus::Different },
                expected: Some(expected.into()),
                actual: Some(actual.into()),
                affects_semantics: false,
                explanation: "Platform differences are recorded for context; they do not alone invalidate input identity.".into(),
            });
        }
        let gcc_requested = self.execution.requested_backend == "sono_gcc";
        let gcc_toolchain_available = !gcc_requested || gcc_toolchain_detected();
        let gcc_identity_matches = if gcc_requested {
            let (current_codegen, current_abi) = crate::sono_gcc::compiler_identity();
            let codegen_matches =
                self.execution.gcc_codegen_version.as_deref() == Some(current_codegen);
            let abi_matches = self.execution.gcc_runtime_abi.as_deref() == Some(current_abi);
            for (component, expected, actual, matches) in [
                (
                    "Sono-GCC code generation version",
                    self.execution.gcc_codegen_version.as_deref(),
                    Some(current_codegen),
                    codegen_matches,
                ),
                (
                    "Sono-GCC runtime ABI",
                    self.execution.gcc_runtime_abi.as_deref(),
                    Some(current_abi),
                    abi_matches,
                ),
            ] {
                items.push(VerificationItem {
                    component: component.into(),
                    category: DifferenceCategory::BackendEnvironment,
                    status: if expected.is_none() {
                        VerificationStatus::Unknown
                    } else if matches {
                        VerificationStatus::Match
                    } else {
                        VerificationStatus::Different
                    },
                    expected: expected.map(str::to_owned),
                    actual: actual.map(str::to_owned),
                    affects_semantics: true,
                    explanation: if expected.is_none() {
                        "This compiler identity was not captured for the original execution.".into()
                    } else if matches {
                        "Compiler/runtime identity matches the recorded execution.".into()
                    } else {
                        "Compiler/runtime identity differs from the recorded execution.".into()
                    },
                });
            }
            codegen_matches && abi_matches
        } else {
            true
        };
        let exact_backend_available = gcc_toolchain_available && gcc_identity_matches;
        items.push(VerificationItem {
            component: "Requested Watch backend".into(),
            category: DifferenceCategory::BackendEnvironment,
            status: if !gcc_toolchain_available {
                VerificationStatus::Different
            } else if !gcc_identity_matches {
                if self.execution.gcc_codegen_version.is_none()
                    || self.execution.gcc_runtime_abi.is_none()
                {
                    VerificationStatus::Unknown
                } else {
                    VerificationStatus::Different
                }
            } else {
                VerificationStatus::Match
            },
            expected: Some(self.execution.requested_backend.clone()),
            actual: Some(if gcc_requested && !gcc_toolchain_available {
                "unavailable on this platform/toolchain"
            } else {
                self.execution.requested_backend.as_str()
            }.into()),
            affects_semantics: true,
            explanation: if !gcc_toolchain_available {
                "Sono-GCC uses the Windows native compiler/loading path; this platform/toolchain cannot provide it. Exact backend reproduction is unavailable, and the renderer will not silently queue a VM substitute.".into()
            } else if !gcc_identity_matches {
                "The original Sono-GCC compiler identity is missing or differs, so exact backend reproduction cannot be established.".into()
            } else {
                "The requested backend and its recorded compiler identity are available; runtime fallback, if any, will be recorded by the new history entry.".into()
            },
        });
        if !self.extra.is_empty() {
            items.push(VerificationItem {
                component: "Unknown manifest fields".into(),
                category: DifferenceCategory::SemanticConfiguration,
                status: VerificationStatus::Unknown,
                expected: Some(self.extra.keys().cloned().collect::<Vec<_>>().join(", ")),
                actual: None,
                affects_semantics: true,
                explanation: "Fields from a newer or extended schema are preserved, but their semantic significance is unknown.".into(),
            });
        }
        let config_valid = match self.effective_config.validate() {
            Ok(()) => true,
            Err(error) => {
                items.push(VerificationItem {
                    component: "Effective RenderConfig".into(),
                    category: DifferenceCategory::SemanticConfiguration,
                    status: VerificationStatus::Mismatch,
                    expected: Some("valid configuration".into()),
                    actual: Some(format!("{error:#}")),
                    affects_semantics: true,
                    explanation: "The stored effective configuration fails current validation."
                        .into(),
                });
                false
            }
        };
        let has_unknown = !self.extra.is_empty();
        let can_reproduce_semantic_inputs = all_inputs_match
            && config_valid
            && !has_unknown
            && version_match
            && commit_matches
            && semantic_match;
        let strongest_stored_reference = if self.references.audio_pcm_hash_available {
            "raw RGB frame sequence and pre-AAC audio PCM reference".into()
        } else if self.references.raw_rgb_sequence_available {
            "raw RGB frame sequence reference".into()
        } else {
            "resolved semantic inputs/configuration only".into()
        };
        items.push(VerificationItem {
            component: "Final encoded media byte identity".into(),
            category: DifferenceCategory::OutputEncoding,
            status: VerificationStatus::NotApplicable,
            expected: None,
            actual: None,
            affects_semantics: false,
            explanation: "The manifest does not claim byte-identical encoded output across codec, muxer, metadata, driver, and platform differences.".into(),
        });
        Ok(ReproducibilityReport {
            manifest_sha256: self.manifest_sha256.clone(),
            schema_supported: true,
            can_reproduce_exact_backend: can_reproduce_semantic_inputs && exact_backend_available,
            can_reproduce_semantic_inputs,
            strongest_stored_reference,
            items,
        })
    }

    pub fn build_reproduction_config(
        &self,
        overrides: &BTreeMap<String, PathBuf>,
        output: &Path,
    ) -> Result<RenderConfig> {
        if output.as_os_str().is_empty() {
            bail!("a new output path is required for manifest reproduction");
        }
        if output.exists() {
            bail!(
                "refusing to replace existing output file: {}",
                output.display()
            );
        }
        let report = self.verify(overrides)?;
        if !report.can_reproduce_exact_backend {
            let differences = report
                .items
                .iter()
                .filter(|item| {
                    matches!(
                        item.status,
                        VerificationStatus::Missing
                            | VerificationStatus::Mismatch
                            | VerificationStatus::Unknown
                            | VerificationStatus::Different
                    ) && item.affects_semantics
                })
                .map(|item| format!("{}: {}", item.component, item.explanation))
                .collect::<Vec<_>>();
            bail!(
                "manifest cannot be reproduced with the exact recorded backend/configuration:\n{}",
                differences.join("\n")
            );
        }
        let mut config = crate::render_history::sanitize_config_snapshot(&self.effective_config);
        apply_overrides(&mut config, &self.inputs, overrides)?;
        config.notifications.enabled = false;
        config.output = Some(output.to_path_buf());
        config.validate_export_inputs()?;
        Ok(config)
    }
}

pub fn compare(left: &RenderManifest, right: &RenderManifest) -> Result<ManifestComparison> {
    left.validate()?;
    right.validate()?;
    let mut differences = Vec::new();
    let left_inputs = left
        .inputs
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect::<BTreeMap<_, _>>();
    let right_inputs = right
        .inputs
        .iter()
        .map(|entry| (&entry.id, entry))
        .collect::<BTreeMap<_, _>>();
    let input_ids = left_inputs
        .keys()
        .chain(right_inputs.keys())
        .copied()
        .collect::<BTreeSet<_>>();
    let mut input_content_matches = true;
    for id in input_ids {
        let a = left_inputs
            .get(id)
            .and_then(|entry| entry.sha256.as_deref());
        let b = right_inputs
            .get(id)
            .and_then(|entry| entry.sha256.as_deref());
        if a != b {
            input_content_matches = false;
            differences.push(ManifestDifference {
                category: DifferenceCategory::InputContent,
                component: id.clone(),
                left: a.map(str::to_owned),
                right: b.map(str::to_owned),
            });
        }
    }

    let left_config = comparable_config(left)?;
    let right_config = comparable_config(right)?;
    let mut semantic_config_matches = true;
    collect_config_differences(
        "",
        &left_config,
        &right_config,
        &mut differences,
        &mut semantic_config_matches,
    );
    for (category, component, a, b) in [
        (
            DifferenceCategory::BackendEnvironment,
            "renderer.version",
            Some(left.renderer.version.clone()),
            Some(right.renderer.version.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "renderer.commit",
            left.renderer.commit.clone(),
            right.renderer.commit.clone(),
        ),
        (
            DifferenceCategory::SemanticConfiguration,
            "renderer.runtime_semantics",
            Some(left.renderer.runtime_semantics.clone()),
            Some(right.renderer.runtime_semantics.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.requested_backend",
            Some(left.execution.requested_backend.clone()),
            Some(right.execution.requested_backend.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.effective_backend",
            Some(left.execution.effective_backend.clone()),
            Some(right.execution.effective_backend.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.target_os",
            Some(left.execution.target_os.clone()),
            Some(right.execution.target_os.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.target_architecture",
            Some(left.execution.target_architecture.clone()),
            Some(right.execution.target_architecture.clone()),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.gcc_codegen_version",
            left.execution.gcc_codegen_version.clone(),
            right.execution.gcc_codegen_version.clone(),
        ),
        (
            DifferenceCategory::BackendEnvironment,
            "execution.gcc_runtime_abi",
            left.execution.gcc_runtime_abi.clone(),
            right.execution.gcc_runtime_abi.clone(),
        ),
        (
            DifferenceCategory::Provenance,
            "profile",
            left.provenance.profile_name.clone(),
            right.provenance.profile_name.clone(),
        ),
        (
            DifferenceCategory::Provenance,
            "project",
            left.provenance.project_name_hint.clone(),
            right.provenance.project_name_hint.clone(),
        ),
    ] {
        if a != b {
            differences.push(ManifestDifference {
                category,
                component: component.into(),
                left: a,
                right: b,
            });
        }
    }
    if left.output.exists != right.output.exists {
        differences.push(ManifestDifference {
            category: DifferenceCategory::OutputEncoding,
            component: "output.exists".into(),
            left: Some(left.output.exists.to_string()),
            right: Some(right.output.exists.to_string()),
        });
    }
    if left.output.size_bytes != right.output.size_bytes {
        differences.push(ManifestDifference {
            category: DifferenceCategory::OutputEncoding,
            component: "output.size_bytes".into(),
            left: left.output.size_bytes.map(|value| value.to_string()),
            right: right.output.size_bytes.map(|value| value.to_string()),
        });
    }
    let raw_frame_reference_matches = compare_optional_reference(
        left.references.rgb_sequence_sha1.as_deref(),
        right.references.rgb_sequence_sha1.as_deref(),
        "references.rgb_sequence_sha1",
        DifferenceCategory::OutputEncoding,
        &mut differences,
    );
    let audio_reference_matches = compare_optional_reference(
        left.references.sfx_pcm_sha1.as_deref(),
        right.references.sfx_pcm_sha1.as_deref(),
        "references.sfx_pcm_sha1",
        DifferenceCategory::OutputEncoding,
        &mut differences,
    );
    compare_optional_reference(
        left.references.sfx_event_sha1.as_deref(),
        right.references.sfx_event_sha1.as_deref(),
        "references.sfx_event_sha1",
        DifferenceCategory::SemanticConfiguration,
        &mut differences,
    );
    Ok(ManifestComparison {
        left_sha256: left.manifest_sha256.clone(),
        right_sha256: right.manifest_sha256.clone(),
        input_content_matches,
        semantic_config_matches,
        raw_frame_reference_matches,
        audio_reference_matches,
        differences,
    })
}

fn compare_optional_reference(
    left: Option<&str>,
    right: Option<&str>,
    component: &str,
    category: DifferenceCategory,
    differences: &mut Vec<ManifestDifference>,
) -> Option<bool> {
    match (left, right) {
        (Some(a), Some(b)) => {
            if a != b {
                differences.push(ManifestDifference {
                    category,
                    component: component.into(),
                    left: Some(a.into()),
                    right: Some(b.into()),
                });
            }
            Some(a == b)
        }
        _ => None,
    }
}

fn comparable_config(manifest: &RenderManifest) -> Result<Value> {
    let mut config = serde_json::to_value(&manifest.effective_config)?;
    normalize_config_asset_paths(&mut config, &manifest.inputs)?;
    if let Some(object) = config.as_object_mut() {
        object.insert("output".into(), Value::Null);
        object.remove("notifications");
        object.remove("job_manager");
    }
    Ok(canonical_value(config))
}

fn collect_config_differences(
    prefix: &str,
    left: &Value,
    right: &Value,
    differences: &mut Vec<ManifestDifference>,
    semantic_config_matches: &mut bool,
) {
    if left == right {
        return;
    }
    match (left, right) {
        (Value::Object(a), Value::Object(b)) => {
            let keys = a.keys().chain(b.keys()).collect::<BTreeSet<_>>();
            for key in keys {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                match (a.get(key), b.get(key)) {
                    (Some(a), Some(b)) => collect_config_differences(
                        &path,
                        a,
                        b,
                        differences,
                        semantic_config_matches,
                    ),
                    (a, b) => {
                        push_config_difference(&path, a, b, differences, semantic_config_matches)
                    }
                }
            }
        }
        _ => push_config_difference(
            prefix,
            Some(left),
            Some(right),
            differences,
            semantic_config_matches,
        ),
    }
}

fn push_config_difference(
    path: &str,
    left: Option<&Value>,
    right: Option<&Value>,
    differences: &mut Vec<ManifestDifference>,
    semantic_config_matches: &mut bool,
) {
    let root = path.split('.').next().unwrap_or(path);
    let category = match root {
        "ui" | "watermark" => DifferenceCategory::Presentation,
        "fps" | "width" | "height" | "start_time" | "duration" | "whole_chart" | "video_crf"
        | "video_preset" | "backend" | "mv_enabled" | "mv_background" => {
            DifferenceCategory::OutputEncoding
        }
        _ => DifferenceCategory::SemanticConfiguration,
    };
    *semantic_config_matches = false;
    differences.push(ManifestDifference {
        category,
        component: format!("config.{path}"),
        left: left.map(compact_value),
        right: right.map(compact_value),
    });
}

fn compact_value(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        _ => value.to_string(),
    }
}

fn verify_item(
    input: &ManifestInput,
    path: Option<&Path>,
    actual: Option<String>,
    status: VerificationStatus,
    explanation: &str,
) -> VerificationItem {
    VerificationItem {
        component: input.role.clone(),
        category: DifferenceCategory::InputContent,
        status,
        expected: input.sha256.clone(),
        actual: actual.or_else(|| path.map(|path| path.display().to_string())),
        affects_semantics: true,
        explanation: explanation.into(),
    }
}

fn stable_file_identity(path: &Path) -> Result<(String, u64)> {
    let before =
        fs::metadata(path).with_context(|| format!("reading asset metadata {}", path.display()))?;
    if !before.is_file() {
        bail!("manifest asset is not a regular file: {}", path.display());
    }
    let identity = sha256_file(path)?;
    let after = fs::metadata(path)
        .with_context(|| format!("rechecking asset metadata {}", path.display()))?;
    let before_modified = before.modified().ok();
    let after_modified = after.modified().ok();
    if before.len() != after.len() || identity.1 != after.len() || before_modified != after_modified
    {
        bail!(
            "asset changed while it was being hashed: {}",
            path.display()
        );
    }
    Ok((format!("sha256:{}", identity.0), identity.1))
}

fn input_specs(config: &RenderConfig) -> Vec<InputSpec> {
    let mut specs = vec![
        spec("engine", "Engine package", true, &config.engine),
        spec("resources", "Resource package", true, &config.resources),
        spec("level", "Level/chart", true, &config.level),
    ];
    if let Some(path) = &config.music {
        specs.push(spec("music", "Music", config.layers.bgm, path));
    }
    if let Some(path) = &config.mv {
        specs.push(spec("mv", "Video background", config.mv_enabled, path));
    }
    for (id, role, path, active) in [
        (
            "resource_override.particles",
            "Particle resource override",
            config.resource_overrides.particles.as_ref(),
            config.layers.particles,
        ),
        (
            "resource_override.effects",
            "Effect resource override",
            config.resource_overrides.effects.as_ref(),
            config.layers.sfx,
        ),
        (
            "resource_override.background",
            "Background resource override",
            config.resource_overrides.background.as_ref(),
            true,
        ),
    ] {
        if let Some(path) = path {
            specs.push(spec(id, role, active, path));
        }
    }
    let custom_ui_active =
        config.ui.enabled && config.ui.mode == crate::render_ui::GameplayUiMode::Custom;
    if let Some(path) = &config.ui.asset_pack {
        specs.push(spec(
            "ui.asset-pack",
            "Gameplay UI asset-pack manifest",
            custom_ui_active,
            path,
        ));
    }
    if let Some(path) = &config.ui.events_file {
        specs.push(spec(
            "ui.events",
            "Gameplay UI event data",
            custom_ui_active,
            path,
        ));
    }
    for (key, path) in &config.ui.assets {
        let resolved_path = config.ui.asset_path_for_manifest(key).or_else(|| {
            let reference = path.to_string_lossy();
            (!reference.starts_with("pack://")
                && (path.is_absolute() || is_foreign_absolute_path(path)))
            .then(|| path.clone())
        });
        specs.push(optional_spec(
            &format!("ui.asset:{key}"),
            &format!("Gameplay UI asset {key}"),
            custom_ui_active,
            resolved_path,
        ));
    }
    for (key, path) in &config.watermark.assets {
        specs.push(spec(
            &format!("watermark.asset:{key}"),
            &format!("Watermark asset {key}"),
            config.watermark.enabled,
            path,
        ));
    }
    specs
}

fn spec(id: &str, role: &str, active: bool, path: &Path) -> InputSpec {
    InputSpec {
        id: id.into(),
        role: role.into(),
        active,
        path: Some(path.to_path_buf()),
    }
}

fn optional_spec(id: &str, role: &str, active: bool, path: Option<PathBuf>) -> InputSpec {
    InputSpec {
        id: id.into(),
        role: role.into(),
        active,
        path,
    }
}

fn config_pointer(id: &str) -> String {
    match id {
        "engine" | "resources" | "level" | "music" | "mv" => format!("/{id}"),
        "resource_override.particles" => "/resource_overrides/particles".into(),
        "resource_override.effects" => "/resource_overrides/effects".into(),
        "resource_override.background" => "/resource_overrides/background".into(),
        "ui.events" => "/ui/events_file".into(),
        "ui.asset-pack" => "/ui/asset_pack".into(),
        value if value.starts_with("ui.asset:") => {
            format!("/ui/assets/{}", escape_pointer(&value[9..]))
        }
        value if value.starts_with("watermark.asset:") => {
            format!("/watermark/assets/{}", escape_pointer(&value[16..]))
        }
        _ => String::new(),
    }
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn normalize_config_asset_paths(config: &mut Value, inputs: &[ManifestInput]) -> Result<()> {
    for input in inputs {
        let pointer = config_pointer(&input.id);
        if pointer.is_empty() {
            continue;
        }
        let replacement = if input.active {
            input
                .sha256
                .as_deref()
                .map(str::to_owned)
                .unwrap_or_else(|| "active-input-without-identity".into())
        } else {
            "inactive-input".into()
        };
        set_pointer(config, &pointer, Value::String(replacement));
    }
    // Asset-root is only a resolver hint. The resolved files and pack
    // manifest have their own content identities, so a relocated root must
    // not change portable manifest identity.
    set_pointer(config, "/ui/asset_root", Value::Null);
    Ok(())
}

fn set_pointer(value: &mut Value, pointer: &str, replacement: Value) {
    let Some((parents, last)) = pointer.rsplit_once('/') else {
        return;
    };
    let key = last.replace("~1", "/").replace("~0", "~");
    let mut current = value;
    for component in parents
        .trim_start_matches('/')
        .split('/')
        .filter(|part| !part.is_empty())
    {
        let decoded = component.replace("~1", "/").replace("~0", "~");
        let Some(next) = current.get_mut(&decoded) else {
            return;
        };
        current = next;
    }
    if let Some(object) = current.as_object_mut() {
        if object.contains_key(&key) {
            object.insert(key, replacement);
        }
    }
}

fn apply_overrides(
    config: &mut RenderConfig,
    inputs: &[ManifestInput],
    overrides: &BTreeMap<String, PathBuf>,
) -> Result<()> {
    let known = inputs
        .iter()
        .map(|input| input.id.as_str())
        .collect::<HashSet<_>>();
    if let Some(unknown) = overrides.keys().find(|key| !known.contains(key.as_str())) {
        bail!("manifest has no asset slot named {unknown:?}");
    }
    for (id, path) in overrides {
        match id.as_str() {
            "engine" => config.engine = path.clone(),
            "resources" => config.resources = path.clone(),
            "level" => config.level = path.clone(),
            "music" => config.music = Some(path.clone()),
            "mv" => config.mv = Some(path.clone()),
            "resource_override.particles" => {
                config.resource_overrides.particles = Some(path.clone())
            }
            "resource_override.effects" => config.resource_overrides.effects = Some(path.clone()),
            "resource_override.background" => {
                config.resource_overrides.background = Some(path.clone())
            }
            "ui.events" => config.ui.events_file = Some(path.clone()),
            value if value.starts_with("ui.asset:") => {
                let key = &value[9..];
                let slot = config
                    .ui
                    .assets
                    .get_mut(key)
                    .with_context(|| format!("config has no UI asset slot {key:?}"))?;
                *slot = path.clone();
            }
            value if value.starts_with("watermark.asset:") => {
                let key = &value[16..];
                let slot = config
                    .watermark
                    .assets
                    .get_mut(key)
                    .with_context(|| format!("config has no watermark asset slot {key:?}"))?;
                *slot = path.clone();
            }
            _ => bail!("manifest has an unsupported asset slot {id:?}"),
        }
    }
    Ok(())
}

fn canonical_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let mut sorted = Map::new();
            let mut values = values.into_iter().collect::<Vec<_>>();
            values.sort_by(|a, b| a.0.cmp(&b.0));
            for (key, value) in values {
                sorted.insert(key, canonical_value(value));
            }
            Value::Object(sorted)
        }
        value => value,
    }
}

fn validate_sha256(value: &str) -> Result<()> {
    let Some(raw) = value.strip_prefix("sha256:") else {
        bail!("manifest input hash must use sha256:<64 lowercase hex> representation");
    };
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("manifest input hash must use sha256:<64 lowercase hex> representation");
    }
    Ok(())
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn sanitize_manifest_config(config: &mut RenderConfig) {
    config.notifications = crate::notifications::NotificationConfig {
        enabled: false,
        ..Default::default()
    };
    config.job_manager = crate::job_config::JobManagerConfig::default();
}

fn is_foreign_absolute_path(path: &Path) -> bool {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        value.starts_with('/') && !value.starts_with("//")
    }
    #[cfg(not(windows))]
    {
        value.starts_with("\\\\")
            || value.starts_with("//")
            || value.as_bytes().get(1) == Some(&b':')
                && value
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphabetic)
    }
}

fn gcc_toolchain_detected() -> bool {
    #[cfg(windows)]
    {
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        std::process::Command::new(rustc)
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

pub fn export_history_atomic(
    history: &crate::render_history::RenderHistoryStore,
    history_id: &str,
    output: &Path,
) -> Result<RenderManifest> {
    let record = history.get(history_id)?;
    let performance = record.performance_record_id.as_deref().and_then(|id| {
        crate::performance::PerformanceStore::open_default()
            .ok()?
            .get(id)
            .ok()
    });
    let manifest = RenderManifest::from_history_with_performance(&record, performance.as_ref())?;
    manifest.save_atomic(output)?;
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn history_with_inputs(root: &Path) -> RenderHistoryRecord {
        for name in [
            "engine.zip",
            "resources.srp",
            "level.json",
            "music.mp3",
            "ui.png",
        ] {
            fs::write(root.join(name), format!("{name}-contents")).unwrap();
        }
        let mut record = RenderHistoryRecord {
            id: "history-job-attempt-1".into(),
            schema_version: crate::render_history::HISTORY_SCHEMA_VERSION,
            job_id: "job".into(),
            renderer_version: env!("CARGO_PKG_VERSION").into(),
            operating_system: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            execution_backend: "Sono VM".into(),
            ..Default::default()
        };
        record.config_snapshot.engine = root.join("engine.zip");
        record.config_snapshot.resources = root.join("resources.srp");
        record.config_snapshot.level = root.join("level.json");
        record.config_snapshot.music = Some(root.join("music.mp3"));
        record.config_snapshot.output = Some(root.join("result.mp4"));
        record.config_snapshot.notifications.discord.bot_token_env =
            "A_PRIVATE_TOKEN_MUST_NOT_APPEAR_IN_A_MANIFEST".into();
        record.config_snapshot.ui = crate::render_ui::RendererUiConfig::default();
        record.config_snapshot.ui.enabled = true;
        record
            .config_snapshot
            .ui
            .assets
            .insert("logo".into(), root.join("ui.png"));
        record.output.path = record.config_snapshot.output.clone();
        record.status = crate::render_history::HistoryStatus::Completed;
        record.validate().unwrap();
        record
    }

    #[test]
    fn manifest_round_trip_hashes_inputs_and_redacts_application_settings() {
        let dir = tempdir().unwrap();
        let record = history_with_inputs(dir.path());
        let manifest = RenderManifest::from_history(&record).unwrap();
        assert!(manifest.inputs.iter().all(|input| input.sha256.is_some()));
        assert!(!manifest.effective_config.notifications.enabled);
        assert!(!manifest.extra.contains_key("bot_token"));
        let path = dir.path().join("render.sonomanifest.json");
        manifest.save_atomic(&path).unwrap();
        assert!(!fs::read_to_string(&path)
            .unwrap()
            .contains("A_PRIVATE_TOKEN_MUST_NOT_APPEAR_IN_A_MANIFEST"));
        let loaded = RenderManifest::load(&path).unwrap();
        assert_eq!(manifest.manifest_sha256, loaded.manifest_sha256);
        assert!(
            loaded
                .verify(&BTreeMap::new())
                .unwrap()
                .can_reproduce_exact_backend
        );
        assert!(loaded.save_atomic(&path).is_err());
    }

    #[test]
    fn pack_asset_history_resolves_hashes_and_relocates_without_path_identity() {
        let original = tempdir().unwrap();
        let original_assets = original.path().join("assets");
        fs::create_dir_all(&original_assets).unwrap();
        let asset_bytes = b"presentation image bytes";
        fs::write(original_assets.join("logo.bin"), asset_bytes).unwrap();
        let pack_path = original.path().join("presentation-pack.json");
        let pack = crate::presentation_assets::PresentationAssetPack {
            id: "manifest-fixture".into(),
            name: "Manifest fixture".into(),
            assets: vec![crate::presentation_assets::PresentationAssetEntry {
                id: "logo".into(),
                source: "logo.bin".into(),
                kind: crate::presentation_assets::PresentationAssetKind::Image,
                ..Default::default()
            }],
            ..Default::default()
        };
        fs::write(&pack_path, serde_json::to_vec(&pack).unwrap()).unwrap();

        let mut record = history_with_inputs(original.path());
        record.config_snapshot.ui.enabled = true;
        record.config_snapshot.ui = crate::render_ui::RendererUiConfig::default();
        record.config_snapshot.ui.enabled = true;
        record.config_snapshot.ui.asset_root = Some(original_assets.clone());
        record.config_snapshot.ui.asset_pack = Some(pack_path.clone());
        record
            .config_snapshot
            .ui
            .assets
            .insert("logo".into(), "pack://logo".into());
        // History persists the durable references but omits the runtime path
        // cache populated while a render profile is loaded.
        record.config_snapshot =
            serde_json::from_value(serde_json::to_value(&record.config_snapshot).unwrap()).unwrap();

        let first = RenderManifest::from_history(&record).unwrap();
        let asset_input = first
            .inputs
            .iter()
            .find(|input| input.id == "ui.asset:logo")
            .unwrap();
        assert_eq!(
            asset_input.sha256,
            Some(format!("sha256:{}", sha256_hex(asset_bytes)))
        );
        assert_eq!(
            asset_input.path_hint.as_deref(),
            Some(
                original_assets
                    .join("logo.bin")
                    .canonicalize()
                    .unwrap()
                    .as_path()
            )
        );
        assert_eq!(
            first.effective_config.ui.assets.get("logo"),
            Some(&PathBuf::from("pack://logo"))
        );

        let relocated = tempdir().unwrap();
        let relocated_assets = relocated.path().join("moved-assets");
        fs::create_dir_all(&relocated_assets).unwrap();
        let relocated_logo = relocated_assets.join("logo.bin");
        fs::copy(original_assets.join("logo.bin"), &relocated_logo).unwrap();
        let relocated_pack = relocated.path().join("presentation-pack.json");
        fs::copy(&pack_path, &relocated_pack).unwrap();
        let mut moved_record = record.clone();
        moved_record.config_snapshot.ui.asset_pack = Some(relocated_pack.clone());
        moved_record.config_snapshot.ui.asset_root = Some(relocated_assets);
        moved_record.output.path = Some(relocated.path().join("output.mp4"));
        moved_record.config_snapshot.output = moved_record.output.path.clone();
        let second = RenderManifest::from_history(&moved_record).unwrap();
        assert_eq!(first.manifest_sha256, second.manifest_sha256);

        let overrides = BTreeMap::from([
            ("ui.asset-pack".to_owned(), relocated_pack),
            ("ui.asset:logo".to_owned(), relocated_logo),
        ]);
        assert!(
            first
                .verify(&overrides)
                .unwrap()
                .can_reproduce_exact_backend
        );

        let mut unresolved_record = record;
        unresolved_record
            .config_snapshot
            .ui
            .assets
            .insert("logo".into(), "pack://not-in-pack".into());
        let unresolved = RenderManifest::from_history(&unresolved_record).unwrap();
        let unresolved_input = unresolved
            .inputs
            .iter()
            .find(|input| input.id == "ui.asset:logo")
            .unwrap();
        assert!(unresolved_input.path_hint.is_none());
        assert!(unresolved_input.sha256.is_none());
        assert!(unresolved
            .verify(&BTreeMap::new())
            .unwrap()
            .items
            .iter()
            .any(|item| item.component == "Gameplay UI asset logo"
                && item.status == VerificationStatus::Missing));

        unresolved_record
            .config_snapshot
            .ui
            .assets
            .insert("relative".into(), "logo.bin".into());
        let unresolved = RenderManifest::from_history(&unresolved_record).unwrap();
        let unresolved_relative = unresolved
            .inputs
            .iter()
            .find(|input| input.id == "ui.asset:relative")
            .unwrap();
        assert!(unresolved_relative.path_hint.is_none());
        assert!(unresolved_relative.sha256.is_none());
        assert!(unresolved
            .verify(&BTreeMap::new())
            .unwrap()
            .items
            .iter()
            .any(|item| item.component == "Gameplay UI asset relative"
                && item.status == VerificationStatus::Missing));
    }

    #[test]
    fn path_changes_do_not_change_content_identity_but_modified_and_missing_assets_report() {
        let root = tempdir().unwrap();
        let record = history_with_inputs(root.path());
        let mut first = RenderManifest::from_history(&record).unwrap();
        let alternate = tempdir().unwrap();
        for input in &first.inputs {
            let source = input.path_hint.as_ref().unwrap();
            fs::copy(source, alternate.path().join(source.file_name().unwrap())).unwrap();
        }
        let relocated_assets = first
            .inputs
            .iter()
            .map(|input| {
                let source = input.path_hint.as_ref().unwrap();
                (
                    input.id.clone(),
                    alternate.path().join(source.file_name().unwrap()),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let relocated_verification = first.verify(&relocated_assets).unwrap();
        assert!(relocated_verification.can_reproduce_exact_backend);
        assert!(relocated_verification
            .items
            .iter()
            .filter(
                |item| item.category == DifferenceCategory::InputContent && item.affects_semantics
            )
            .all(|item| item.status == VerificationStatus::Match));
        let relocated_config = first
            .build_reproduction_config(
                &relocated_assets,
                &alternate.path().join("relocated-rerender.mp4"),
            )
            .unwrap();
        assert_eq!(relocated_config.engine, alternate.path().join("engine.zip"));

        let mut moved = record.clone();
        moved.config_snapshot.engine = alternate.path().join("engine.zip");
        moved.config_snapshot.resources = alternate.path().join("resources.srp");
        moved.config_snapshot.level = alternate.path().join("level.json");
        moved.config_snapshot.music = Some(alternate.path().join("music.mp3"));
        moved
            .config_snapshot
            .ui
            .assets
            .insert("logo".into(), alternate.path().join("ui.png"));
        moved.output.path = Some(alternate.path().join("new-output.mp4"));
        moved.config_snapshot.output = moved.output.path.clone();
        let second = RenderManifest::from_history(&moved).unwrap();
        assert_eq!(
            comparable_config(&first).unwrap(),
            comparable_config(&second).unwrap()
        );
        assert_eq!(first.manifest_sha256, second.manifest_sha256);

        let engine = first
            .inputs
            .iter()
            .find(|input| input.id == "engine")
            .unwrap();
        fs::write(engine.path_hint.as_ref().unwrap(), b"changed").unwrap();
        let report = first.verify(&BTreeMap::new()).unwrap();
        assert!(report
            .items
            .iter()
            .any(|item| item.component == "Engine package"
                && item.status == VerificationStatus::Mismatch));
        first.inputs[0].path_hint = Some(root.path().join("removed.zip"));
        first.seal().unwrap();
        let report = first.verify(&BTreeMap::new()).unwrap();
        assert!(report
            .items
            .iter()
            .any(|item| item.status == VerificationStatus::Missing));
    }

    #[test]
    fn unknown_future_fields_are_preserved_and_block_exact_reproduction() {
        let dir = tempdir().unwrap();
        let manifest = RenderManifest::from_history(&history_with_inputs(dir.path())).unwrap();
        let mut value = serde_json::to_value(&manifest).unwrap();
        value["future_semantics"] = json!({"mode": "unknown"});
        let mut manifest: RenderManifest = serde_json::from_value(value).unwrap();
        manifest.seal().unwrap();
        assert!(manifest
            .verify(&BTreeMap::new())
            .unwrap()
            .items
            .iter()
            .any(|item| item.status == VerificationStatus::Unknown));
        assert!(manifest.extra.contains_key("future_semantics"));
    }

    #[test]
    fn windows_path_hints_remain_hints_when_opened_on_unix() {
        #[cfg(not(windows))]
        {
            let dir = tempdir().unwrap();
            let mut manifest =
                RenderManifest::from_history(&history_with_inputs(dir.path())).unwrap();
            let foreign = PathBuf::from(r"C:\Charts\engine.zip");
            manifest
                .inputs
                .iter_mut()
                .find(|input| input.id == "engine")
                .unwrap()
                .path_hint = Some(foreign.clone());
            manifest.effective_config.engine = foreign.clone();
            manifest.seal().unwrap();
            let path = dir.path().join("foreign-path.json");
            manifest.save_atomic(&path).unwrap();
            let loaded = RenderManifest::load(&path).unwrap();
            assert_eq!(loaded.effective_config.engine, foreign);
            assert_eq!(
                loaded
                    .inputs
                    .iter()
                    .find(|input| input.id == "engine")
                    .unwrap()
                    .path_hint
                    .as_ref(),
                Some(&PathBuf::from(r"C:\Charts\engine.zip"))
            );
            assert!(loaded
                .verify(&BTreeMap::new())
                .unwrap()
                .items
                .iter()
                .any(|item| item.component == "Engine package"
                    && item.status == VerificationStatus::Missing));
        }
    }

    #[test]
    fn diff_reports_input_presentation_encoding_and_backend_sections() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        let mut left = RenderManifest::from_history(&history_with_inputs(dir_a.path())).unwrap();
        let mut right = RenderManifest::from_history(&history_with_inputs(dir_b.path())).unwrap();
        fs::write(dir_b.path().join("engine.zip"), b"different engine").unwrap();
        right
            .inputs
            .iter_mut()
            .find(|input| input.id == "engine")
            .unwrap()
            .sha256 = Some(format!("sha256:{}", sha256_hex(b"different engine")));
        right.effective_config.width = 1280;
        right.effective_config.ui.profile_name = "changed hud".into();
        right.execution.requested_backend = "sono_gcc".into();
        left.seal().unwrap();
        right.seal().unwrap();
        let report = compare(&left, &right).unwrap();
        assert!(!report.input_content_matches);
        assert!(report
            .differences
            .iter()
            .any(|item| item.category == DifferenceCategory::InputContent));
        assert!(report
            .differences
            .iter()
            .any(|item| item.category == DifferenceCategory::OutputEncoding));
        assert!(report
            .differences
            .iter()
            .any(|item| item.category == DifferenceCategory::Presentation));
        assert!(report
            .differences
            .iter()
            .any(|item| item.category == DifferenceCategory::BackendEnvironment));
    }
}
