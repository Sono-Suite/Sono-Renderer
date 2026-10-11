//! Evidence-based static and bounded dynamic engine/project compatibility analysis.
//!
//! Static findings describe what the package contains and what this build
//! implements. A function's presence in a graph does not prove that callback
//! reaches or executes it. The optional dynamic pass exercises only the
//! bounded Watch preprocess system and reports that exact scope.

use crate::{formats, runtime, sono_gcc, watch, watch_runtime};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::Read,
    path::Path,
};
use zip::ZipArchive;

const REPORT_SCHEMA_VERSION: u32 = 2;
const DYNAMIC_MAX_GRAPH_NODES: usize = 100_000;
const DYNAMIC_MAX_LEVEL_ENTITIES: usize = 64;
const DYNAMIC_CALLBACK_EVALUATION_CAP: usize = 100_000;
const DYNAMIC_MAX_RUNTIME_ENTITIES: usize = 4_096;
const MAX_REPORTED_OPERATION_NODES: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineReport {
    pub sonolus_version: Option<u64>,
    pub archetypes: Vec<String>,
    pub functions_operators: Vec<String>,
    pub memory_blocks: Vec<String>,
    pub skin_bindings: Value,
    pub effect_bindings: Value,
    pub particle_bindings: Value,
    pub required_resources: Vec<String>,
    /// Kept for compatibility with the original inspector's JSON contract.
    pub currently_unsupported_runtime_features: Vec<String>,
    pub support: LayerSupport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerSupport {
    pub parse: String,
    pub vm_runtime: String,
    pub rendering: String,
    pub particles: String,
    pub audio: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// Legacy fields remain present so existing `compatibility` command users
    /// can continue consuming the original report shape.
    pub engine: EngineReport,
    pub level_entity_count: usize,
    pub level_archetypes: Vec<LevelArchetypeStatus>,
    pub resources: formats::ScpIndex,
    pub required_resource_status: Vec<ResourceStatus>,
    pub resource_integrity_issues: Vec<String>,
    /// Versioned, evidence-oriented extension to the legacy inspector.
    pub schema_version: u32,
    pub assessments: Vec<CategoryAssessment>,
    pub operations: Vec<OperationFinding>,
    pub structural_findings: Vec<StructuralFinding>,
    pub dynamic_validation: Option<DynamicValidation>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompatibilityStatus {
    Supported,
    Unsupported,
    Partial,
    FallbackRequired,
    DynamicallyVerified,
    UnknownRuntimeDependent,
}

impl CompatibilityStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Supported => "Supported",
            Self::Unsupported => "Unsupported",
            Self::Partial => "Partial",
            Self::FallbackRequired => "Fallback required",
            Self::DynamicallyVerified => "Dynamically verified (exercised scope only)",
            Self::UnknownRuntimeDependent => "Unknown / runtime dependent",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryAssessment {
    pub category: String,
    pub status: CompatibilityStatus,
    pub evidence: Vec<String>,
    pub warnings: Vec<String>,
    pub fallback: Option<String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationFinding {
    pub function: String,
    /// Total static WatchData nodes with this function name. Reachability and
    /// whether the node executes are deliberately not inferred here.
    pub occurrence_count: usize,
    /// Bounded example list; `occurrence_count` remains the complete total.
    pub example_node_indices: Vec<usize>,
    pub vm_implementation: runtime::WatchOperationImplementation,
    pub gcc_eligible_occurrences: usize,
    pub vm_fallback_occurrences: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuralFinding {
    pub severity: FindingSeverity,
    pub code: String,
    pub message: String,
    pub node_index: Option<usize>,
    pub entity_index: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FindingSeverity {
    Information,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceStatus {
    pub category: String,
    pub name: String,
    pub resolved: bool,
    pub integrity: ResourceIntegrity,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResourceIntegrity {
    Validated,
    Missing,
    Malformed,
    OptionalMissing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LevelArchetypeStatus {
    pub archetype: Value,
    pub engine_provides: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicValidation {
    pub outcome: DynamicOutcome,
    pub exercised: Vec<String>,
    pub level_entities: usize,
    pub watch_graph_nodes: usize,
    pub runtime_entity_limit: usize,
    pub callback_evaluation_limit: usize,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DynamicOutcome {
    PreprocessCompleted,
    ExecutionFailed,
    SkippedSafetyLimit,
}

fn report(path: &Path) -> Result<(formats::EnginePackage, EngineReport)> {
    let pkg = formats::load_engine(path)?;
    let inv = watch::inventory(&pkg.watch);
    let required = pkg
        .metadata
        .resource_defaults()
        .into_iter()
        .map(|(_, name)| name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let report = EngineReport {
        sonolus_version: pkg.metadata.version,
        archetypes: inv.archetypes,
        functions_operators: inv.functions,
        memory_blocks: inv.memory_blocks,
        skin_bindings: inv.skin_bindings,
        effect_bindings: inv.effect_bindings,
        particle_bindings: inv.particle_bindings,
        required_resources: required,
        currently_unsupported_runtime_features: vec![
            "Static graph presence does not prove callback reachability or execution".into(),
            "Sonolus engine format version acceptance is not explicitly range-gated".into(),
            "Stack operations, Random/RandomInteger, and StreamSet are supported with documented compatibility approximations; exact Sonolus client parity is unverified".into(),
            "Curved Draw has a tessellated renderer path, but client pixel parity is not validated".into(),
        ],
        support: LayerSupport {
            parse: "supported for recognized package envelopes; unknown fields retained".into(),
            vm_runtime: "operation support is reported per function; lifecycle and host behavior remain scope-dependent".into(),
            rendering: "partial SkinData, background, and ordinary/curved engine Watch Draw rendering; curved client pixel parity remains unverified".into(),
            particles: "partial ParticleData effects and atlas rendering; seeded randomness is deterministic host policy".into(),
            audio: "partial offline PCM16 mono/stereo effect mixing; real-time audio device playback is not implemented".into(),
        },
    };
    Ok((pkg, report))
}

pub fn inspect_engine(path: &Path) -> Result<EngineReport> {
    Ok(report(path)?.1)
}

pub fn analyze(
    engine: &Path,
    resource_path: &Path,
    level_path: &Path,
) -> Result<CompatibilityReport> {
    analyze_with_options(engine, resource_path, level_path, false)
}

/// Analyze static package evidence, optionally executing one bounded
/// single-threaded Watch preprocess pass. The dynamic mode refuses inputs
/// above its fixed caps and never invokes Sono-GCC or a renderer/export path.
pub fn analyze_with_options(
    engine: &Path,
    resource_path: &Path,
    level_path: &Path,
    dynamic: bool,
) -> Result<CompatibilityReport> {
    let (pkg, engine_report) = report(engine)?;
    let level = formats::load_level(level_path)?;
    let (resources, resource_parse_error) = match formats::inspect_scp(resource_path) {
        Ok(resources) => (resources, None),
        Err(error) => (
            formats::ScpIndex {
                resources: Vec::new(),
            },
            Some(format!("{error:#}")),
        ),
    };
    let integrity_result = formats::verify_scp_references(resource_path);
    let resource_integrity_issues = match &integrity_result {
        Ok(issues) => issues.clone(),
        Err(error) => vec![format!("SCP integrity scan failed: {error:#}")],
    };

    let known: BTreeSet<String> = pkg
        .level_archetype_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let mut level_archetypes = BTreeSet::new();
    for entity in &level.entities {
        level_archetypes.insert(entity.archetype.to_string());
    }
    let level_archetypes = level_archetypes
        .into_iter()
        .map(|s| {
            let archetype = serde_json::from_str::<Value>(&s).unwrap_or(Value::String(s));
            let name = archetype.as_str().unwrap_or("").to_owned();
            LevelArchetypeStatus {
                engine_provides: known.contains(&name)
                    || matches!(name.as_str(), "#BPM_CHANGE" | "#TIMESCALE_CHANGE"),
                archetype,
            }
        })
        .collect::<Vec<_>>();

    let required_resource_status = pkg
        .metadata
        .resource_defaults()
        .into_iter()
        .map(|(category, name)| {
            let resolved = resources
                .resources
                .iter()
                .any(|r| r.category == category && r.name == name);
            let (integrity, detail) = if !resolved {
                match selected_manifest_exists(resource_path, &category, &name) {
                    Ok(true) => (ResourceIntegrity::Malformed, Some("a manifest exists at the selected resource path but it did not form a matching valid SCP index entry".into())),
                    Ok(false) => (ResourceIntegrity::Missing, Some("required engine default is absent from the selected SCP".into())),
                    Err(error) => (ResourceIntegrity::Malformed, Some(format!("could not inspect selected resource manifest: {error:#}"))),
                }
            } else {
                validate_selected_resource(resource_path, &category, &name)
            };
            ResourceStatus { category, name, resolved, integrity, detail }
        })
        .collect::<Vec<_>>();

    let gcc_coverage = sono_gcc::inspect_operation_coverage(&pkg.watch.nodes);
    let mut operations_by_name: BTreeMap<String, OperationFinding> = BTreeMap::new();
    for (index, node) in pkg.watch.nodes.iter().enumerate() {
        let Some(function) = node.func.as_deref() else {
            continue;
        };
        let finding = operations_by_name
            .entry(function.to_owned())
            .or_insert_with(|| OperationFinding {
                function: function.to_owned(),
                occurrence_count: 0,
                example_node_indices: Vec::new(),
                vm_implementation: runtime::watch_operation_implementation(function),
                gcc_eligible_occurrences: 0,
                vm_fallback_occurrences: 0,
            });
        finding.occurrence_count += 1;
        if finding.example_node_indices.len() < MAX_REPORTED_OPERATION_NODES {
            finding.example_node_indices.push(index);
        }
        if gcc_coverage.get(index).copied().unwrap_or(false) {
            finding.gcc_eligible_occurrences += 1;
        } else {
            finding.vm_fallback_occurrences += 1;
        }
    }
    let operations = operations_by_name.into_values().collect::<Vec<_>>();
    let structural_findings = inspect_structure(&pkg.watch, &level);
    let dynamic_validation = dynamic.then(|| run_bounded_dynamic(&pkg, resource_path, &level));
    let assessments = build_assessments(
        &pkg,
        &level,
        &resources,
        resource_parse_error.as_deref(),
        &required_resource_status,
        &resource_integrity_issues,
        &level_archetypes,
        &operations,
        &structural_findings,
        dynamic_validation.as_ref(),
    );

    Ok(CompatibilityReport {
        engine: engine_report,
        level_entity_count: level.entities.len(),
        level_archetypes,
        resources,
        required_resource_status,
        resource_integrity_issues,
        schema_version: REPORT_SCHEMA_VERSION,
        assessments,
        operations,
        structural_findings,
        dynamic_validation,
    })
}

fn validate_selected_resource(
    path: &Path,
    category: &str,
    name: &str,
) -> (ResourceIntegrity, Option<String>) {
    if category == "effects" {
        return match formats::load_effect_assets(path, name) {
            Ok(assets) => validate_effect_assets(&assets),
            Err(error) => (ResourceIntegrity::Malformed, Some(format!("{error:#}"))),
        };
    }
    let result = match category {
        "skins" => formats::load_skin_assets(path, name).map(|_| ()),
        "backgrounds" => formats::load_background_assets(path, name).map(|_| ()),
        // Compatibility analysis is the explicit strict diagnostic path. A
        // normal render remains permissive and can omit an isolated bad effect.
        "particles" => formats::load_particle_assets_with_mode(
            path,
            name,
            formats::ParticleValidationMode::Strict,
        )
        .map(|_| ()),
        _ => {
            return (
                ResourceIntegrity::Unknown,
                Some("no validator is registered for this resource category".into()),
            )
        }
    };
    match result {
        Ok(()) => (ResourceIntegrity::Validated, None),
        Err(error) => (ResourceIntegrity::Malformed, Some(format!("{error:#}"))),
    }
}

fn validate_effect_assets(assets: &formats::EffectAssets) -> (ResourceIntegrity, Option<String>) {
    let missing_audio = assets.clips.values().filter(|clip| clip.is_none()).count();
    if missing_audio == 0 {
        (ResourceIntegrity::Validated, None)
    } else {
        (
            ResourceIntegrity::OptionalMissing,
            Some(format!(
                "effect data is valid, but {missing_audio} optional clip audio asset(s) are unavailable"
            )),
        )
    }
}

fn selected_manifest_exists(path: &Path, category: &str, name: &str) -> Result<bool> {
    let file = File::open(path)?;
    let mut archive = ZipArchive::new(file)?;
    let manifest = format!("sonolus/{category}/{name}");
    let result = match archive.by_name(&manifest) {
        Ok(mut entry) => {
            // Read only a small prefix. Existence is sufficient here; the
            // ordinary resource loader performs structural and hash checks.
            let mut prefix = [0_u8; 1];
            let _ = entry.read(&mut prefix)?;
            Ok(true)
        }
        Err(zip::result::ZipError::FileNotFound) => Ok(false),
        Err(error) => Err(error.into()),
    };
    result
}

fn inspect_structure(
    watch_data: &watch::WatchData,
    level: &formats::LevelData,
) -> Vec<StructuralFinding> {
    let mut findings = Vec::new();
    for (index, node) in watch_data.nodes.iter().enumerate() {
        match (&node.func, &node.value) {
            (Some(_), Some(_)) => findings.push(StructuralFinding {
                severity: FindingSeverity::Warning,
                code: "node_has_function_and_value".into(),
                message:
                    "Watch node contains both func and value; runtime interpretation is ambiguous"
                        .into(),
                node_index: Some(index),
                entity_index: None,
            }),
            (None, None) => findings.push(StructuralFinding {
                severity: FindingSeverity::Error,
                code: "node_missing_function_and_value".into(),
                message: "Watch node has neither func nor value".into(),
                node_index: Some(index),
                entity_index: None,
            }),
            (None, Some(value)) if value.as_f64().is_none() => {
                findings.push(StructuralFinding {
                    severity: FindingSeverity::Error,
                    code: "node_value_not_numeric".into(),
                    message: "constant Watch node value is not numeric".into(),
                    node_index: Some(index),
                    entity_index: None,
                });
            }
            _ => {}
        }
        for (arg_index, arg) in node.args.iter().enumerate() {
            let Some(child) = arg.as_u64() else {
                findings.push(StructuralFinding {
                    severity: FindingSeverity::Error,
                    code: "node_argument_not_index".into(),
                    message: format!("argument {arg_index} is not a nonnegative node index"),
                    node_index: Some(index),
                    entity_index: None,
                });
                continue;
            };
            if child as usize >= watch_data.nodes.len() {
                findings.push(StructuralFinding {
                    severity: FindingSeverity::Error,
                    code: "node_argument_out_of_range".into(),
                    message: format!(
                        "argument {arg_index} references node {child}, but graph has {} nodes",
                        watch_data.nodes.len()
                    ),
                    node_index: Some(index),
                    entity_index: None,
                });
            }
        }
    }
    let mut archetype_names = BTreeSet::new();
    for (archetype_index, archetype) in watch_data.archetypes.iter().enumerate() {
        if archetype.name.is_empty() || !archetype_names.insert(archetype.name.as_str()) {
            findings.push(StructuralFinding {
                severity: FindingSeverity::Error,
                code: if archetype.name.is_empty() { "empty_engine_archetype_name" } else { "duplicate_engine_archetype_name" }.into(),
                message: format!("engine Watch archetype at index {archetype_index} has an empty or duplicate name {:?}", archetype.name),
                node_index: None,
                entity_index: None,
            });
        }
        for (callback_name, callback) in [
            ("preprocess", archetype.preprocess.as_ref()),
            ("spawnTime", archetype.spawn_time.as_ref()),
            ("despawnTime", archetype.despawn_time.as_ref()),
            ("initialize", archetype.initialize.as_ref()),
            ("updateSequential", archetype.update_sequential.as_ref()),
            ("updateParallel", archetype.update_parallel.as_ref()),
            ("terminate", archetype.terminate.as_ref()),
        ] {
            if let Some(callback) = callback {
                let node_index = callback.get("index").and_then(Value::as_u64);
                if node_index.is_none_or(|node| node as usize >= watch_data.nodes.len()) {
                    findings.push(StructuralFinding {
                        severity: FindingSeverity::Error,
                        code: "callback_node_out_of_range".into(),
                        message: format!("archetype {:?} {callback_name} callback does not reference a valid Watch node", archetype.name),
                        node_index: node_index.map(|node| node as usize),
                        entity_index: None,
                    });
                }
            }
        }
    }
    if watch_data
        .update_spawn
        .is_some_and(|node| node >= watch_data.nodes.len())
    {
        findings.push(StructuralFinding {
            severity: FindingSeverity::Error,
            code: "update_spawn_node_out_of_range".into(),
            message: "Watch updateSpawn does not reference a valid Watch node".into(),
            node_index: watch_data.update_spawn,
            entity_index: None,
        });
    }
    for (entity_index, entity) in level.entities.iter().enumerate() {
        if !entity.archetype.is_string() {
            findings.push(StructuralFinding {
                severity: FindingSeverity::Error,
                code: "level_archetype_not_string".into(),
                message: "level entity archetype must be a string".into(),
                node_index: None,
                entity_index: Some(entity_index),
            });
        }
    }
    findings
}

fn run_bounded_dynamic(
    pkg: &formats::EnginePackage,
    resource_path: &Path,
    level: &formats::LevelData,
) -> DynamicValidation {
    let watch_data = &pkg.watch;
    let skipped = |detail: String| DynamicValidation {
        outcome: DynamicOutcome::SkippedSafetyLimit,
        exercised: Vec::new(),
        level_entities: level.entities.len(),
        watch_graph_nodes: watch_data.nodes.len(),
        runtime_entity_limit: DYNAMIC_MAX_RUNTIME_ENTITIES,
        callback_evaluation_limit: DYNAMIC_CALLBACK_EVALUATION_CAP,
        detail: Some(detail),
    };
    if watch_data.nodes.len() > DYNAMIC_MAX_GRAPH_NODES {
        return skipped(format!(
            "Watch graph has {} nodes; dynamic limit is {DYNAMIC_MAX_GRAPH_NODES}",
            watch_data.nodes.len()
        ));
    }
    if level.entities.len() > DYNAMIC_MAX_LEVEL_ENTITIES {
        return skipped(format!(
            "Level has {} entities; dynamic limit is {DYNAMIC_MAX_LEVEL_ENTITIES}",
            level.entities.len()
        ));
    }
    let mut runtime = match watch_runtime::WatchRuntime::new(watch_data, level) {
        Ok(runtime) => runtime,
        Err(error) => {
            return DynamicValidation {
                outcome: DynamicOutcome::ExecutionFailed,
                exercised: vec![
                    "WatchRuntime::new input binding and runtime-model construction".into(),
                ],
                level_entities: level.entities.len(),
                watch_graph_nodes: watch_data.nodes.len(),
                runtime_entity_limit: DYNAMIC_MAX_RUNTIME_ENTITIES,
                callback_evaluation_limit: DYNAMIC_CALLBACK_EVALUATION_CAP,
                detail: Some(format!("{error:#}")),
            }
        }
    };
    if let Err(error) = runtime.set_parallel_updates(false, None) {
        return skipped(format!(
            "could not select single-threaded Watch execution: {error:#}"
        ));
    }
    runtime.set_compatibility_safety_limits(
        DYNAMIC_CALLBACK_EVALUATION_CAP,
        DYNAMIC_MAX_RUNTIME_ENTITIES,
    );
    let bind_inputs = (|| -> Result<()> {
        runtime.bind_engine_rom(&pkg.rom)?;
        runtime.bind_engine_option_defaults(&pkg.configuration)?;
        runtime.bind_engine_ui_configuration(&pkg.configuration)?;
        let defaults: BTreeMap<_, _> = pkg.metadata.resource_defaults().into_iter().collect();
        let skin_names = defaults
            .get("skins")
            .map(|name| formats::load_skin_sprite_names(resource_path, name))
            .transpose()?
            .unwrap_or_default();
        let effect_names = defaults
            .get("effects")
            .map(|name| formats::load_effect_clip_names(resource_path, name))
            .transpose()?
            .unwrap_or_default();
        let particle_names = defaults
            .get("particles")
            .map(|name| formats::load_particle_effect_names(resource_path, name))
            .transpose()?
            .unwrap_or_default();
        runtime.bind_skin_sprite_names(&skin_names)?;
        runtime.bind_effect_clip_names(&effect_names)?;
        runtime.bind_particle_effect_names(&particle_names)?;
        Ok(())
    })();
    if let Err(error) = bind_inputs {
        return DynamicValidation {
            outcome: DynamicOutcome::ExecutionFailed,
            exercised: vec![
                "WatchRuntime::new: archetype/entity/import binding".into(),
                "engine ROM, engine options/UI defaults, and selected skin/effect/particle resource-name binding".into(),
            ],
            level_entities: level.entities.len(),
            watch_graph_nodes: watch_data.nodes.len(),
            runtime_entity_limit: DYNAMIC_MAX_RUNTIME_ENTITIES,
            callback_evaluation_limit: DYNAMIC_CALLBACK_EVALUATION_CAP,
            detail: Some(format!("dynamic input binding failed before Watch execution: {error:#}")),
        };
    }
    match runtime.preprocess() {
        Ok(()) => DynamicValidation {
            outcome: DynamicOutcome::PreprocessCompleted,
            exercised: vec![
                "WatchRuntime::new: archetype/entity/import binding".into(),
                "engine ROM, engine options/UI defaults, and selected skin/effect/particle resource-name binding".into(),
                format!("Watch preprocess callbacks at most once per supplied level entity ({} entities)", level.entities.len()),
                "Sono VM interpreter, single-threaded; no frame/update/render/export work".into(),
            ],
            level_entities: level.entities.len(),
            watch_graph_nodes: watch_data.nodes.len(),
            runtime_entity_limit: DYNAMIC_MAX_RUNTIME_ENTITIES,
            callback_evaluation_limit: DYNAMIC_CALLBACK_EVALUATION_CAP,
            detail: None,
        },
        Err(error) => DynamicValidation {
            outcome: DynamicOutcome::ExecutionFailed,
            exercised: vec![
                "WatchRuntime::new: archetype/entity/import binding".into(),
                "Watch preprocess callbacks until completion or a runtime/safety error".into(),
            ],
            level_entities: level.entities.len(),
            watch_graph_nodes: watch_data.nodes.len(),
            runtime_entity_limit: DYNAMIC_MAX_RUNTIME_ENTITIES,
            callback_evaluation_limit: DYNAMIC_CALLBACK_EVALUATION_CAP,
            detail: Some(format!("{error:#}")),
        },
    }
}

fn build_assessments(
    pkg: &formats::EnginePackage,
    level: &formats::LevelData,
    _resources: &formats::ScpIndex,
    resource_parse_error: Option<&str>,
    required: &[ResourceStatus],
    integrity_issues: &[String],
    level_archetypes: &[LevelArchetypeStatus],
    operations: &[OperationFinding],
    structure: &[StructuralFinding],
    dynamic: Option<&DynamicValidation>,
) -> Vec<CategoryAssessment> {
    let mut assessments = Vec::new();
    let leveldata_round_trip = serde_json::to_vec(level)
        .and_then(|bytes| serde_json::from_slice::<formats::LevelData>(&bytes))
        .map(|decoded| decoded == *level)
        .unwrap_or(false);
    assessments.push(CategoryAssessment {
        category: "Chart Studio authoring".into(),
        status: if leveldata_round_trip {
            CompatibilityStatus::Supported
        } else {
            CompatibilityStatus::Partial
        },
        evidence: vec![
            format!("Raw LevelData authoring: supported by the canonical entity model for {} structurally represented entities; generic create/edit/delete/reorder operations do not require archetype gameplay semantics", level.entities.len()),
            format!("Canonical JSON parse/serialize round-trip for this loaded LevelData: {}", if leveldata_round_trip { "passed" } else { "failed" }),
            "Authoring schema: not evaluated by this compatibility command".into(),
            "High-level editor: Generic / Raw Mode is the only claim made by this report".into(),
            "Preview: Chart Studio uses the production RenderConfig::render_frame and Watch pipeline; this compatibility analysis does not execute that preview".into(),
            "Runtime: see separate Watch VM and Rendering assessments; this structural authoring result does not upgrade runtime compatibility".into(),
        ],
        warnings: vec!["This compatibility analysis does not load an optional authoring schema. High-level semantic authoring is not inferred from Watch metadata; use Generic / Raw Mode unless a matching schema is loaded in Chart Studio.".into()],
        fallback: Some("Edit canonical entity data in Generic / Raw Mode. A content-matched declarative authoring schema can add labels, constraints, creation templates, and explicit timing bindings.".into()),
        limitations: vec!["This category does not prove that edited LevelData is accepted by every engine callback, that entity relationships are valid, or that the engine has a meaningful lane/beat visualization. Use the real Watch preview and runtime compatibility findings.".into()],
    });
    assessments.extend([
        CategoryAssessment {
            category: "Chart Studio raw LevelData authoring".into(),
            status: CompatibilityStatus::Supported,
            evidence: vec![format!(
                "The production LevelData parser accepted this document with {} entities; canonical entity JSON fields are retained independently of archetype-specific semantics",
                level.entities.len()
            )],
            warnings: Vec::new(),
            fallback: None,
            limitations: vec!["This is structural data support; it does not establish that arbitrary newly created entities satisfy engine callbacks or cross-entity relationships.".into()],
        },
        CategoryAssessment {
            category: "Chart Studio LevelData round-trip".into(),
            status: if leveldata_round_trip {
                CompatibilityStatus::Supported
            } else {
                CompatibilityStatus::Partial
            },
            evidence: vec![format!(
                "This loaded document {} a canonical serde JSON parse/serialize comparison",
                if leveldata_round_trip { "passed" } else { "failed" }
            )],
            warnings: Vec::new(),
            fallback: None,
            limitations: vec!["This analyzer comparison is structural and in-memory; it is not a byte-identity claim or a substitute for standalone gzip export/reimport testing.".into()],
        },
        CategoryAssessment {
            category: "Chart Studio authoring schema".into(),
            status: CompatibilityStatus::UnknownRuntimeDependent,
            evidence: vec!["Optional external authoring schema was not supplied to this analyzer and was not loaded or identity-checked".into()],
            warnings: vec!["No schema status can be inferred from EngineData alone. A stale or mismatched schema must be rejected by Chart Studio and fall back to raw mode.".into()],
            fallback: Some("Use Generic / Raw Mode, or explicitly load a schema bound to this engine's semantic content hash.".into()),
            limitations: vec!["Schema availability and declared authoring constraints do not establish Sonolus runtime requirements.".into()],
        },
        CategoryAssessment {
            category: "Chart Studio high-level editor".into(),
            status: CompatibilityStatus::Partial,
            evidence: vec!["High-level controls require an explicitly loaded, content-matched declarative schema; this analysis does not load one".into()],
            warnings: vec!["This report makes no claim that gameplay concepts can be inferred from archetype names, JSON field names, or Watch graph behavior.".into()],
            fallback: Some("Generic / Raw Mode remains the schema-independent authoring path.".into()),
            limitations: vec!["A property inspector is not a universal spatial chart, relationship editor, or semantic gameplay validator.".into()],
        },
        CategoryAssessment {
            category: "Chart Studio preview".into(),
            status: CompatibilityStatus::UnknownRuntimeDependent,
            evidence: vec!["Chart Studio delegates frame preview to the production RenderConfig::render_frame path and authoritative Watch runtime".into()],
            warnings: vec!["This static compatibility analysis did not execute a Chart Studio preview for the edited document.".into()],
            fallback: None,
            limitations: vec!["Preview success is scoped to the exact engine, LevelData, resources, time, and renderer build exercised; static analysis cannot promise it.".into()],
        },
    ]);
    let unknown_archetypes = level_archetypes
        .iter()
        .filter(|a| !a.engine_provides)
        .count();
    assessments.push(CategoryAssessment {
        category: "Engine and level loading".into(),
        status: if structure.iter().any(|f| f.severity == FindingSeverity::Error) || unknown_archetypes > 0 { CompatibilityStatus::Unsupported } else { CompatibilityStatus::Supported },
        evidence: vec![
            format!("Engine package parsed; Watch graph contains {} nodes and {} archetypes", pkg.watch.nodes.len(), pkg.watch.archetypes.len()),
            format!("LevelData parsed with {} entities", level.entities.len()),
            format!("Engine metadata version field: {}", pkg.metadata.version.map_or_else(|| "absent".into(), |v| v.to_string())),
        ],
        warnings: (unknown_archetypes > 0).then(|| format!("{unknown_archetypes} distinct level archetype(s) are not declared by this engine; host-generated #BPM_CHANGE/#TIMESCALE_CHANGE are accepted. Constructor validation will clarify the first mismatch.")).into_iter().collect(),
        fallback: None,
        limitations: vec!["No explicit supported range for Sonolus format versions is enforced; the metadata version field is not interpreted as a renderer/compiler version.".into()],
    });

    let unsupported = operations
        .iter()
        .filter(|op| op.vm_implementation == runtime::WatchOperationImplementation::Unsupported)
        .count();
    let approximate = operations
        .iter()
        .filter(|op| op.vm_implementation == runtime::WatchOperationImplementation::Approximate)
        .count();
    let unknown = operations
        .iter()
        .filter(|op| op.vm_implementation == runtime::WatchOperationImplementation::Unknown)
        .count();
    let malformed_watch_node = structure
        .iter()
        .any(|finding| finding.severity == FindingSeverity::Error && finding.node_index.is_some());
    assessments.push(CategoryAssessment {
        category: "Watch VM".into(),
        status: if malformed_watch_node {
            CompatibilityStatus::Unsupported
        } else if unsupported > 0 || approximate > 0 || unknown > 0 {
            CompatibilityStatus::Partial
        } else {
            CompatibilityStatus::Supported
        },
        evidence: vec![
            format!("{} distinct operation names classified from the VM implementation capability registry", operations.len()),
            format!("{} names use compatibility approximations; {} names are explicitly unsupported; {} remain unknown to the registry", approximate, unsupported, unknown),
        ],
        warnings: {
            let mut warnings = Vec::new();
            if approximate > 0 {
                warnings.push("Approximate operation presence is static evidence only. If reached at runtime, these operations emit a job warning; exact official-client behavior is not claimed.".into());
            }
            if unsupported > 0 || unknown > 0 {
                warnings.push("Operation presence is static evidence only: reachability, selected branches, callback execution, and runtime argument validity are not proven.".into());
            }
            warnings
        },
        fallback: None,
        limitations: vec!["Dynamically verified means only the exercised preprocess pass completed; it is not full engine compatibility.".into()],
    });

    let gcc_nodes: usize = operations.iter().map(|o| o.gcc_eligible_occurrences).sum();
    let fallback_nodes: usize = operations.iter().map(|o| o.vm_fallback_occurrences).sum();
    let gcc_platform_available = gcc_platform_supported(std::env::consts::OS);
    assessments.push(CategoryAssessment {
        category: "Sono-GCC".into(),
        status: if !gcc_platform_available || fallback_nodes > 0 { CompatibilityStatus::FallbackRequired } else { CompatibilityStatus::Supported },
        evidence: vec![
            format!("{} Watch operation nodes are eligible for scalar native regions; {} require VM fallback or are outside the compiler subset", gcc_nodes, fallback_nodes),
            format!("Current target platform: {}; native DLL compilation implementation is {}", std::env::consts::OS, if gcc_platform_available { "available on Windows" } else { "Windows-only and unavailable on this target" }),
            format!("Compiler backend identity: {:?}", sono_gcc::compiler_identity()),
        ],
        warnings: vec!["Eligibility is static graph coverage, not a promise that compilation/toolchain loading will succeed. Cache state is observed only when precompile is requested.".into()],
        fallback: Some("Noncompiled operations and unavailable native compilation continue through the Sono VM, where supported.".into()),
        limitations: vec!["This analyzer does not invoke rustc, compile the graph, or mutate cache state.".into()],
    });

    let has_draw = operations.iter().any(|o| o.function == "Draw");
    let has_curved = operations
        .iter()
        .any(|o| o.function.starts_with("DrawCurved"));
    assessments.push(CategoryAssessment {
        category: "Rendering".into(),
        status: if has_curved { CompatibilityStatus::Partial } else if has_draw { CompatibilityStatus::Partial } else { CompatibilityStatus::UnknownRuntimeDependent },
        evidence: vec![format!("Watch graph {} a standard Draw node", if has_draw { "contains" } else { "does not contain" })],
        warnings: has_curved.then(|| "Curved Draw operations are present but their mesh rasterization path is not implemented.".into()).into_iter().collect(),
        fallback: None,
        limitations: vec!["Static operation inventory does not establish visible frame output, texture/resource resolution, or visual parity.".into()],
    });

    let has_particle = operations
        .iter()
        .any(|o| o.function.contains("ParticleEffect") || o.function == "HasParticleEffect");
    assessments.push(CategoryAssessment {
        category: "Particles".into(),
        status: if has_particle {
            CompatibilityStatus::Partial
        } else {
            CompatibilityStatus::UnknownRuntimeDependent
        },
        evidence: vec![format!(
            "Watch graph {} particle-related operations",
            if has_particle {
                "contains"
            } else {
                "does not contain"
            }
        )],
        warnings: Vec::new(),
        fallback: None,
        limitations: vec![
            "Static presence does not prove resource binding or visual particle simulation output."
                .into(),
        ],
    });

    let has_audio = operations.iter().any(|o| {
        o.function.starts_with("Play")
            || o.function.starts_with("StopLooped")
            || o.function == "HasEffectClip"
    });
    assessments.push(CategoryAssessment {
        category: "SFX and audio".into(),
        status: if has_audio { CompatibilityStatus::Partial } else { CompatibilityStatus::UnknownRuntimeDependent },
        evidence: vec![format!("Watch graph {} audio-related operations", if has_audio { "contains" } else { "does not contain" })],
        warnings: Vec::new(), fallback: None,
        limitations: vec!["VM audio event capture and offline asset mixing do not imply real-time device playback or exact client mixer behavior.".into()],
    });

    let missing = required
        .iter()
        .filter(|r| r.integrity == ResourceIntegrity::Missing)
        .count();
    let malformed = required
        .iter()
        .filter(|r| r.integrity == ResourceIntegrity::Malformed)
        .count();
    let optional_missing = required
        .iter()
        .filter(|r| r.integrity == ResourceIntegrity::OptionalMissing)
        .count();
    assessments.push(CategoryAssessment {
        category: "Resources".into(),
        status: if resource_parse_error.is_some() || missing > 0 || malformed > 0 {
            CompatibilityStatus::Unsupported
        } else if required
            .iter()
            .any(|r| r.integrity == ResourceIntegrity::Unknown)
        {
            CompatibilityStatus::UnknownRuntimeDependent
        } else if optional_missing > 0 || !integrity_issues.is_empty() {
            CompatibilityStatus::Partial
        } else {
            CompatibilityStatus::Supported
        },
        evidence: vec![format!("{} engine-default resource(s) checked", required.len()), format!("{} SCP content-addressed reference integrity issue(s)", integrity_issues.len())],
        warnings: resource_parse_error
            .map(|error| vec![format!("SCP index could not be read: {error}")])
            .or_else(|| (optional_missing > 0 || !integrity_issues.is_empty()).then(|| vec!["Optional selected-resource components are absent or SCP integrity issues were found; details remain attached to the individual resource records.".into()]))
            .unwrap_or_default(),
        fallback: Some("Missing resources can be queried by Watch Has* operations, but output may omit engine visuals or audio.".into()),
        limitations: vec!["Only engine metadata default resources are loaded and deeply validated; arbitrary runtime-selected resource IDs are not inferred.".into()],
    });

    assessments.push(CategoryAssessment {
        category: "Gameplay presentation".into(),
        status: CompatibilityStatus::UnknownRuntimeDependent,
        evidence: vec!["No static proof of client-provided gameplay HUD state or profile-specific visual output is available from engine/project files alone.".into()],
        warnings: Vec::new(), fallback: None,
        limitations: vec!["Requires a rendered frame and external state/event inputs for visual and state-driven presentation validation.".into()],
    });

    assessments.push(CategoryAssessment {
        category: "Platform and export".into(),
        status: CompatibilityStatus::UnknownRuntimeDependent,
        evidence: vec![format!("Analyzer is running on {} / {}", std::env::consts::OS, std::env::consts::ARCH)],
        warnings: Vec::new(), fallback: None,
        limitations: vec!["GPU adapter, FFmpeg availability, output filesystem permissions, and media encoding are not probed by static compatibility analysis.".into()],
    });
    if let Some(dynamic) = dynamic {
        assessments.push(CategoryAssessment {
            category: "Bounded dynamic validation".into(),
            status: if dynamic.outcome == DynamicOutcome::PreprocessCompleted { CompatibilityStatus::DynamicallyVerified } else { CompatibilityStatus::UnknownRuntimeDependent },
            evidence: dynamic.exercised.clone(),
            warnings: dynamic.detail.iter().cloned().collect(),
            fallback: None,
            limitations: vec!["The result applies only to runtime construction and the stated preprocess pass; it does not verify frame/update behavior, all graph nodes, resource rendering, audio output, or export.".into()],
        });
    }
    assessments
}

fn gcc_platform_supported(target_os: &str) -> bool {
    target_os.eq_ignore_ascii_case("windows")
}

pub fn render_human_readable(report: &CompatibilityReport) -> String {
    let mut out = format!(
        "Compatibility analysis (schema {})\nEngine: {}\nLevel entities: {}\n",
        report.schema_version,
        report.engine.sonolus_version.map_or_else(
            || "version unknown".into(),
            |v| format!("metadata version {v}")
        ),
        report.level_entity_count
    );
    for assessment in &report.assessments {
        out.push_str(&format!(
            "\n{}: {}\n",
            assessment.category,
            assessment.status.label()
        ));
        for evidence in &assessment.evidence {
            out.push_str(&format!("  Evidence: {evidence}\n"));
        }
        for warning in &assessment.warnings {
            out.push_str(&format!("  Warning: {warning}\n"));
        }
        if let Some(fallback) = &assessment.fallback {
            out.push_str(&format!("  Fallback: {fallback}\n"));
        }
        for limitation in &assessment.limitations {
            out.push_str(&format!("  Limitation: {limitation}\n"));
        }
    }
    out.push_str(&format!(
        "\nWatch operation names: {}\nStructural findings: {}\n",
        report.operations.len(),
        report.structural_findings.len()
    ));
    for operation in &report.operations {
        out.push_str(&format!("  {}: VM {:?}; GCC eligible {}; VM-routed (support classified separately) {}; occurrences {}\n", operation.function, operation.vm_implementation, operation.gcc_eligible_occurrences, operation.vm_fallback_occurrences, operation.occurrence_count));
    }
    if let Some(dynamic) = &report.dynamic_validation {
        out.push_str(&format!(
            "\nBounded dynamic validation: {:?}\n",
            dynamic.outcome
        ));
        for item in &dynamic.exercised {
            out.push_str(&format!("  Exercised: {item}\n"));
        }
        if let Some(detail) = &dynamic.detail {
            out.push_str(&format!("  Detail: {detail}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, io::Write};
    use zip::{write::SimpleFileOptions, ZipWriter};

    fn tiny_engine_and_inputs(
        root: &Path,
        function: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        tiny_engine_and_inputs_with_metadata(root, function, r#"{"version":1}"#)
    }

    fn tiny_engine_and_inputs_with_metadata(
        root: &Path,
        function: &str,
        metadata: &str,
    ) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let engine = root.join("engine.zip");
        let file = fs::File::create(&engine).unwrap();
        let mut zip = ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        for (name, data) in [
            ("engine.json", metadata.as_bytes()),
            ("EngineConfiguration", br#"{"options":[]}"#.as_slice()),
            (
                "EnginePlayData",
                br#"{"archetypes":[{"name":"A"}]}"#.as_slice(),
            ),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.start_file("EngineWatchData", options).unwrap();
        let watch_data = format!(
            r#"{{"archetypes":[{{"name":"A","preprocess":{{"index":1}}}}],"skin":{{"sprites":[]}},"effect":{{"clips":[]}},"particle":{{"effects":[]}},"nodes":[{{"value":0}},{{"func":"{function}","args":[0]}}]}}"#
        );
        zip.write_all(watch_data.as_bytes()).unwrap();
        zip.finish().unwrap();
        let level = root.join("level.json");
        fs::write(&level, br#"{"entities":[{"archetype":"A","data":[]}] }"#).unwrap();
        let resources = root.join("resources.scp");
        let file = fs::File::create(&resources).unwrap();
        ZipWriter::new(file).finish().unwrap();
        (engine, resources, level)
    }

    #[test]
    fn operation_classifier_distinguishes_supported_missing_and_unknown() {
        assert_eq!(
            runtime::watch_operation_implementation("Add"),
            runtime::WatchOperationImplementation::Supported
        );
        assert_eq!(
            runtime::watch_operation_implementation("DrawCurvedB"),
            runtime::WatchOperationImplementation::Supported
        );
        assert_eq!(
            runtime::watch_operation_implementation("StackPush"),
            runtime::WatchOperationImplementation::Approximate
        );
        assert_eq!(
            runtime::watch_operation_implementation("FutureSonolusOp"),
            runtime::WatchOperationImplementation::Unknown
        );
    }

    #[test]
    fn gcc_platform_check_requires_the_current_windows_backend() {
        assert!(gcc_platform_supported("windows"));
        assert!(!gcc_platform_supported("linux"));
        assert!(!gcc_platform_supported("macos"));
    }

    #[test]
    fn analyzer_keeps_legacy_report_fields_and_marks_unknowns_without_rejecting() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs(temp.path(), "FutureSonolusOp");
        let report = analyze(&engine, &resources, &level).unwrap();
        assert_eq!(report.schema_version, 2);
        assert_eq!(report.level_entity_count, 1);
        assert_eq!(
            report.operations[0].vm_implementation,
            runtime::WatchOperationImplementation::Unknown
        );
        assert!(report
            .assessments
            .iter()
            .any(|a| a.status == CompatibilityStatus::Partial));
        let authoring = report
            .assessments
            .iter()
            .find(|assessment| assessment.category == "Chart Studio authoring")
            .expect("compatibility output exposes structural authoring separately");
        assert_eq!(authoring.status, CompatibilityStatus::Supported);
        assert!(authoring
            .warnings
            .iter()
            .any(|warning| warning.contains("does not load an optional authoring schema")));
        assert!(authoring
            .fallback
            .as_deref()
            .is_some_and(|fallback| fallback.contains("Generic / Raw Mode")));
        for category in [
            "Chart Studio raw LevelData authoring",
            "Chart Studio LevelData round-trip",
            "Chart Studio authoring schema",
            "Chart Studio high-level editor",
            "Chart Studio preview",
        ] {
            assert!(report
                .assessments
                .iter()
                .any(|assessment| assessment.category == category));
        }
        let schema_assessment = report
            .assessments
            .iter()
            .find(|assessment| assessment.category == "Chart Studio authoring schema")
            .unwrap();
        assert_eq!(
            schema_assessment.status,
            CompatibilityStatus::UnknownRuntimeDependent
        );
    }

    #[test]
    fn dynamic_validation_reports_only_preprocess_scope_and_work_caps() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs(temp.path(), "Add");
        let report = analyze_with_options(&engine, &resources, &level, true).unwrap();
        let dynamic = report.dynamic_validation.unwrap();
        assert_eq!(dynamic.outcome, DynamicOutcome::PreprocessCompleted);
        assert!(dynamic
            .exercised
            .iter()
            .any(|step| step.contains("no frame/update/render/export")));

        let oversized: watch::WatchData = serde_json::from_value(
            json!({"nodes": vec![json!({"value":0}); DYNAMIC_MAX_GRAPH_NODES + 1]}),
        )
        .unwrap();
        let oversized_package = formats::EnginePackage {
            root: Default::default(),
            metadata: serde_json::from_value(json!({})).unwrap(),
            configuration: json!({}),
            rom: Vec::new(),
            play: None,
            watch: oversized,
        };
        let skipped = run_bounded_dynamic(
            &oversized_package,
            &temp.path().join("unused.scp"),
            &formats::LevelData {
                bgm_offset: None,
                entities: vec![],
                metadata: None,
                extra: Default::default(),
            },
        );
        assert_eq!(skipped.outcome, DynamicOutcome::SkippedSafetyLimit);
    }

    #[test]
    fn static_structure_reports_bad_node_references_and_archetypes() {
        let watch_data: watch::WatchData =
            serde_json::from_value(json!({"nodes":[{"func":"Add","args":[7]}]})).unwrap();
        let level: formats::LevelData =
            serde_json::from_value(json!({"entities":[{"archetype":3}]})).unwrap();
        let findings = inspect_structure(&watch_data, &level);
        assert!(findings
            .iter()
            .any(|f| f.code == "node_argument_out_of_range"));
        assert!(findings
            .iter()
            .any(|f| f.code == "level_archetype_not_string"));
    }

    #[test]
    fn compiler_ineligible_functions_are_reported_as_vm_routed_without_claiming_validity() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs(temp.path(), "Draw");
        let report = analyze(&engine, &resources, &level).unwrap();
        let draw = report
            .operations
            .iter()
            .find(|operation| operation.function == "Draw")
            .unwrap();
        assert_eq!(draw.gcc_eligible_occurrences, 0);
        assert_eq!(draw.vm_fallback_occurrences, 1);
        assert_eq!(
            draw.vm_implementation,
            runtime::WatchOperationImplementation::Supported
        );
    }

    #[test]
    fn required_missing_and_malformed_resource_manifests_are_distinguished() {
        let missing_dir = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs_with_metadata(
            missing_dir.path(),
            "Add",
            r#"{"version":1,"skin":{"name":"missing-skin"}}"#,
        );
        let missing = analyze(&engine, &resources, &level).unwrap();
        assert_eq!(
            missing.required_resource_status[0].integrity,
            ResourceIntegrity::Missing
        );

        let malformed_dir = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs_with_metadata(
            malformed_dir.path(),
            "Add",
            r#"{"version":1,"skin":{"name":"broken-skin"}}"#,
        );
        let file = fs::File::create(&resources).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("sonolus/skins/broken-skin", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"not-json").unwrap();
        zip.finish().unwrap();
        let malformed = analyze(&engine, &resources, &level).unwrap();
        assert_eq!(
            malformed.required_resource_status[0].integrity,
            ResourceIntegrity::Malformed
        );
    }

    #[test]
    fn optional_missing_effect_audio_is_not_mislabeled_as_a_malformed_resource() {
        let assets = formats::EffectAssets {
            clips: BTreeMap::from([
                ("available".into(), Some(vec![1, 2, 3])),
                ("silent".into(), None),
            ]),
        };
        let (status, detail) = validate_effect_assets(&assets);
        assert_eq!(status, ResourceIntegrity::OptionalMissing);
        assert!(detail.unwrap().contains("optional clip audio"));
    }

    #[test]
    fn unrelated_scp_integrity_issue_is_not_reported_as_required_resource_failure() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs(temp.path(), "Add");
        let file = fs::File::create(&resources).unwrap();
        let mut zip = ZipWriter::new(file);
        zip.start_file("sonolus/skins/unrelated", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(br#"{"item":{"name":"unrelated","data":{"hash":"missing-hash"}}}"#)
            .unwrap();
        zip.finish().unwrap();
        let report = analyze(&engine, &resources, &level).unwrap();
        let resources_status = report
            .assessments
            .iter()
            .find(|assessment| assessment.category == "Resources")
            .unwrap();
        assert_eq!(resources_status.status, CompatibilityStatus::Partial);
        assert_eq!(report.required_resource_status.len(), 0);
        assert_eq!(report.resource_integrity_issues.len(), 1);
    }

    #[test]
    fn dynamic_probe_executes_approximate_watch_operations_and_reports_partial_static_status() {
        let temp = tempfile::tempdir().unwrap();
        let (engine, resources, level) = tiny_engine_and_inputs(temp.path(), "StackPush");
        let report = analyze_with_options(&engine, &resources, &level, true).unwrap();
        assert_eq!(
            report.operations[0].vm_implementation,
            runtime::WatchOperationImplementation::Approximate
        );
        assert_eq!(
            report
                .assessments
                .iter()
                .find(|assessment| assessment.category == "Watch VM")
                .unwrap()
                .status,
            CompatibilityStatus::Partial
        );
        assert_eq!(
            report.dynamic_validation.unwrap().outcome,
            DynamicOutcome::PreprocessCompleted
        );
    }
}
