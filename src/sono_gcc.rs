//! Opt-in native execution for strict Watch scalar regions.
//!
//! WatchVm remains authoritative. Direct native scalar operations are
//! surrounded by ordered VM-evaluated scalar cuts for runtime, memory, control,
//! and effectful nodes; the resulting `f64` values cross into native parents.
//! Those cuts do not reimplement the VM operation or call back from native code.

use crate::watch::EngineNode;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::{Duration, Instant},
};

/// Renderer execution mode shared by CLI, GUI, and the Watch runtime.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WatchExecutionMode {
    #[default]
    Interpreter,
    SonoGcc,
}

const MAX_REGION_EVALUATIONS: usize = 16_384;
const MAX_REGION_DEPTH: usize = 256;
const MAX_NATIVE_SOURCE_BYTES: usize = 64 * 1024 * 1024;
const COMPILER_BACKEND_VERSION: &str = "sono-gcc-codegen-v6";
const COMPILED_RUNTIME_ABI_VERSION: &str = "watchvm-scalar-cut-abi-v2";
const COMPILER_FLAGS_ID: &str = "rust-2021;cdylib;opt-level=3;strict-fp-defaults";

pub fn compiler_identity() -> (&'static str, &'static str) {
    (COMPILER_BACKEND_VERSION, COMPILED_RUNTIME_ABI_VERSION)
}

type NativeRun = unsafe extern "C" fn(*const f64, usize) -> f64;

pub(crate) const NATIVE_FUNCTION_COUNT: usize = 41;
pub(crate) const INLINE_REGION_INPUTS: usize = 8;
pub(crate) const REGION_SIZE_BUCKET_COUNT: usize = 8;
/// Avoid the FFI/input-marshalling path for singleton scalar regions. Such a
/// subtree remains available through WatchVm; the policy is based on the
/// measured dynamic size profile and can be set to 1 to reproduce legacy
/// all-region dispatch.
pub(crate) const MIN_NATIVE_REGION_NODES: usize = 2;

/// Bucket compiled-region sizes by the number of direct native operation
/// nodes. Used only by the opt-in detailed Watch profiler.
pub(crate) fn region_size_bucket(native_nodes: usize) -> usize {
    match native_nodes {
        0..=1 => 0,
        2 => 1,
        3..=4 => 2,
        5..=8 => 3,
        9..=16 => 4,
        17..=32 => 5,
        33..=64 => 6,
        _ => 7,
    }
}

const NATIVE_FUNCTION_NAMES: [&str; NATIVE_FUNCTION_COUNT] = [
    "Abs",
    "Add",
    "Arccos",
    "Arcsin",
    "Arctan",
    "Arctan2",
    "Ceil",
    "Clamp",
    "Cos",
    "Cosh",
    "Degree",
    "Divide",
    "Equal",
    "Floor",
    "Frac",
    "Greater",
    "GreaterOr",
    "Less",
    "LessOr",
    "Log",
    "Max",
    "Min",
    "Multiply",
    "Negate",
    "Not",
    "NotEqual",
    "Power",
    "Radian",
    "Rem",
    "Remap",
    "RemapClamped",
    "Round",
    "Sign",
    "Sin",
    "Sinh",
    "Subtract",
    "Tan",
    "Tanh",
    "Trunc",
    "Unlerp",
    "UnlerpClamped",
];

pub(crate) fn native_function_name(index: usize) -> &'static str {
    NATIVE_FUNCTION_NAMES[index]
}

fn compact_function_counts(counts: &BTreeMap<String, u64>) -> Vec<(u8, u64)> {
    counts
        .iter()
        .map(|(name, count)| {
            let index = NATIVE_FUNCTION_NAMES
                .iter()
                .position(|candidate| *candidate == name)
                .expect("codegen counted an operation without a native function slot");
            (index as u8, *count)
        })
        .collect()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) struct RegionMetrics {
    pub evaluations: usize,
    pub compiled_evaluations: usize,
    pub max_depth: usize,
}

#[derive(Clone)]
pub(crate) struct CompiledRegion {
    root: usize,
    run: NativeRun,
    pub cut_nodes: Vec<usize>,
    pub cut_depths: Vec<usize>,
    pub metrics: RegionMetrics,
    pub function_counts: Vec<(u8, u64)>,
    pub function_dispatch_count: u64,
    pub compiled_nodes: Vec<usize>,
}

impl CompiledRegion {
    pub(crate) fn run(&self, inputs: &[f64]) -> f64 {
        unsafe { (self.run)(inputs.as_ptr(), inputs.len()) }
    }

    pub(crate) fn size_bucket(&self) -> usize {
        region_size_bucket(self.compiled_nodes.len())
    }
}

pub(crate) struct SonoGccProgram {
    region_by_node: Vec<u32>,
    regions: Vec<std::sync::Arc<CompiledRegion>>,
    operation_counts: std::sync::OnceLock<(u64, u64)>,
    native_load_time: Duration,
    #[cfg(windows)]
    _library: NativeLibrary,
    #[cfg(windows)]
    _cache_lease: Option<crate::cache_manager::EntryLock>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct CompileTimings {
    pub cache_lookup: Duration,
    pub compilation: Duration,
    pub region_codegen: Duration,
    pub rustc_compile: Duration,
    pub native_load: Duration,
    pub cache_write: Duration,
    pub graph_identity: Duration,
    pub disk_validation: Duration,
    pub metadata_reconstruction: Duration,
    pub operation_summary: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheHit {
    None,
    Process,
    Persistent,
}

pub(crate) struct SonoGccCompileResult {
    pub program: std::sync::Arc<SonoGccProgram>,
    pub cache_key: String,
    pub cache_hit: CacheHit,
    pub timings: CompileTimings,
    pub graph_nodes: usize,
    pub compiled_operations: u64,
    pub eligible_operations: u64,
    pub cache_write_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SonoGccPrecompileReport {
    pub cache_key: String,
    pub cache_hit: CacheHit,
    pub timings: CompileTimings,
    pub graph_nodes: usize,
    pub compiled_regions: usize,
    pub compiled_operations: u64,
    pub eligible_operations: u64,
    pub cache_write_error: Option<String>,
}

/// Compile an engine Watch graph and populate the shared process/disk cache.
/// The temporary loaded program is retained by the process cache on success.
pub fn precompile(nodes: &[EngineNode]) -> Result<SonoGccPrecompileReport> {
    precompile_with_progress(nodes, || {})
}

/// Compile through the shared cache and notify the caller only after both
/// process and persistent cache lookups miss.
pub fn precompile_with_progress<F>(
    nodes: &[EngineNode],
    on_compile_start: F,
) -> Result<SonoGccPrecompileReport>
where
    F: FnOnce() + Send,
{
    let result = SonoGccProgram::compile_cached_detailed_with_progress(nodes, on_compile_start)?;
    Ok(SonoGccPrecompileReport {
        cache_key: result.cache_key,
        cache_hit: result.cache_hit,
        timings: result.timings,
        graph_nodes: result.graph_nodes,
        compiled_regions: result.program.region_count(),
        compiled_operations: result.compiled_operations,
        eligible_operations: result.eligible_operations,
        cache_write_error: result.cache_write_error,
    })
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SonoGccCoverage {
    pub graph_nodes: usize,
    pub eligible_operations: u64,
}

/// Return a cheap, static estimate for GUI progress without invoking rustc.
pub fn inspect_coverage(nodes: &[EngineNode]) -> SonoGccCoverage {
    SonoGccCoverage {
        graph_nodes: nodes.len(),
        eligible_operations: eligible_operation_count(nodes),
    }
}

/// Per-node static Sono-GCC coverage using the same eligibility classifier
/// consumed by graph compilation. A false result means the node itself is not
/// a native scalar operation; VM-cut children may still feed an eligible
/// parent region.
pub fn inspect_operation_coverage(nodes: &[EngineNode]) -> Vec<bool> {
    let mut states = vec![0; nodes.len()];
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            !is_numeric(node)
                && node
                    .func
                    .as_deref()
                    .is_some_and(|name| !is_vm_cut_function(name))
                && eligible(nodes, index, &mut states, 0)
        })
        .collect()
}

impl SonoGccProgram {
    pub(crate) fn region(&self, node: usize) -> Option<std::sync::Arc<CompiledRegion>> {
        let index = *self.region_by_node.get(node)?;
        if index == u32::MAX {
            None
        } else {
            self.regions.get(index as usize).cloned()
        }
    }

    pub(crate) fn region_count(&self) -> usize {
        self.regions.len()
    }

    fn compiled_operation_count(&self) -> u64 {
        self.regions
            .iter()
            .flat_map(|region| region.compiled_nodes.iter().copied())
            .collect::<HashSet<_>>()
            .len() as u64
    }

    fn operation_counts(&self, nodes: &[EngineNode]) -> (u64, u64) {
        *self.operation_counts.get_or_init(|| {
            (
                self.compiled_operation_count(),
                eligible_operation_count(nodes),
            )
        })
    }

    pub(crate) fn native_load_time(&self) -> Duration {
        self.native_load_time
    }

    pub(crate) fn compile(nodes: &[EngineNode]) -> Result<(Self, Duration, Duration, Duration)> {
        let started = Instant::now();
        #[cfg(not(windows))]
        {
            let _ = nodes;
            bail!("native DLL compilation is currently available only on Windows");
        }
        #[cfg(windows)]
        {
            compile_windows(nodes, started)
        }
    }

    #[cfg(test)]
    pub(crate) fn compile_cached(
        nodes: &[EngineNode],
    ) -> Result<(std::sync::Arc<Self>, Duration, bool)> {
        let result = Self::compile_cached_detailed(nodes)?;
        let elapsed = result
            .timings
            .compilation
            .saturating_add(result.timings.native_load);
        Ok((result.program, elapsed, result.cache_hit != CacheHit::None))
    }

    #[cfg(test)]
    pub(crate) fn compile_cached_detailed(nodes: &[EngineNode]) -> Result<SonoGccCompileResult> {
        Self::compile_cached_detailed_with_progress(nodes, || {})
    }

    /// Return a program only when this process already has the exact graph in
    /// memory. GUI exports use this non-blocking probe while background
    /// precompilation is still warming the graph.
    pub(crate) fn compile_if_process_cached(
        nodes: &[EngineNode],
    ) -> Result<Option<SonoGccCompileResult>> {
        #[cfg(not(windows))]
        {
            let _ = nodes;
            bail!("native DLL compilation is currently available only on Windows")
        }
        #[cfg(windows)]
        {
            let lookup_started = Instant::now();
            let identity = cache_identity(nodes)?;
            let cache_key = identity.digest.as_bytes().to_vec();
            let program = process_cache()
                .lock()
                .map_err(|_| anyhow::anyhow!("Sono-GCC process cache lock poisoned"))?
                .get(&cache_key)
                .cloned();
            Ok(program.map(|program| {
                compile_result(
                    program,
                    identity.digest,
                    CacheHit::Process,
                    CompileTimings {
                        cache_lookup: lookup_started.elapsed(),
                        graph_identity: identity.identity_time,
                        ..CompileTimings::default()
                    },
                    nodes,
                )
            }))
        }
    }

    pub(crate) fn compile_cached_detailed_with_progress<F>(
        nodes: &[EngineNode],
        on_compile_start: F,
    ) -> Result<SonoGccCompileResult>
    where
        F: FnOnce() + Send,
    {
        #[cfg(not(windows))]
        {
            let _ = nodes;
            let _ = on_compile_start;
            bail!("native DLL compilation is currently available only on Windows");
        }
        #[cfg(windows)]
        {
            compile_cached_windows(nodes, on_compile_start)
        }
    }
}

#[derive(Debug)]
struct CompileUnit {
    root: usize,
    expression: String,
    cut_nodes: Vec<usize>,
    cut_depths: Vec<usize>,
    metrics: RegionMetrics,
    function_counts: Vec<(u8, u64)>,
    function_dispatch_count: u64,
    compiled_nodes: Vec<usize>,
}

fn is_numeric(node: &EngineNode) -> bool {
    node.value
        .as_ref()
        .and_then(serde_json::Value::as_f64)
        .is_some()
}

fn child_indices(node: &EngineNode, owner: usize) -> Result<Vec<usize>> {
    node.args
        .iter()
        .map(|arg| {
            arg.as_u64()
                .map(|value| value as usize)
                .with_context(|| format!("invalid child index in Watch node {owner}"))
        })
        .collect()
}

fn used_children(name: &str, children: &[usize]) -> Result<Vec<usize>> {
    let count = match name {
        "Add" | "Multiply" | "Min" | "Max" | "Rem" | "Subtract" | "Divide" | "Power" => {
            if children.is_empty() {
                bail!("{name} requires at least one argument");
            }
            children.len()
        }
        "Equal" | "NotEqual" | "Greater" | "GreaterOr" | "Less" | "LessOr" => {
            if children.len() < 2 {
                bail!("{name} requires at least two arguments");
            }
            2
        }
        "Arctan2" => {
            if children.len() != 2 {
                bail!("Arctan2 requires exactly two arguments");
            }
            2
        }
        "Clamp" | "Unlerp" | "UnlerpClamped" => {
            if children.len() != 3 {
                bail!("{name} requires exactly three arguments for native compilation");
            }
            3
        }
        "Lerp" | "LerpClamped" => {
            if children.len() != 3 {
                bail!("{name} requires exactly three arguments for native compilation");
            }
            3
        }
        "Remap" | "RemapClamped" => {
            if children.len() != 5 {
                bail!("{name} requires exactly five arguments for native compilation");
            }
            5
        }
        "Abs" | "Arctan" | "Ceil" | "Cos" | "Floor" | "Log" | "Negate" | "Not" | "Round"
        | "Sin" | "Trunc" | "Frac" | "Sign" | "Radian" | "Degree" | "Arcsin" | "Arccos" | "Tan"
        | "Cosh" | "Sinh" | "Tanh" => {
            if children.is_empty() {
                bail!("{name} requires an argument");
            }
            1
        }
        _ if ease_parts(name).is_some() && !name.ends_with("Elastic") => {
            if children.len() != 1 {
                bail!("{name} requires exactly one argument");
            }
            1
        }
        _ => bail!("{name} is not a Sono-GCC pure scalar operation"),
    };
    Ok(children[..count].to_vec())
}

// These nodes stay in WatchVm. Their scalar results cross the same ordered
// input boundary as Get values, so native parents can use authoritative
// runtime, control-flow, memory, and effect results without reimplementing
// those operations in the generated DLL.
const VM_CUT_FUNCTIONS: &[&str] = &[
    "And",
    "BeatToBPM",
    "BeatToStartingBeat",
    "BeatToStartingTime",
    "BeatToTime",
    "Block",
    "Break",
    "Copy",
    "DebugLog",
    "DebugPause",
    "DecrementPost",
    "DecrementPostPointed",
    "DecrementPostShifted",
    "DecrementPre",
    "DecrementPrePointed",
    "DecrementPreShifted",
    "DestroyParticleEffect",
    "DoWhile",
    "Draw",
    "EaseInElastic",
    "EaseInOutElastic",
    "EaseOutElastic",
    "EaseOutInElastic",
    "Execute",
    "Get",
    "GetPointed",
    "GetShifted",
    "HasEffectClip",
    "HasParticleEffect",
    "HasSkinSprite",
    "If",
    "IncrementPost",
    "IncrementPostPointed",
    "IncrementPostShifted",
    "IncrementPre",
    "IncrementPrePointed",
    "IncrementPreShifted",
    "JumpLoop",
    "Lerp",
    "LerpClamped",
    "Mod",
    "MoveParticleEffect",
    "Or",
    "Play",
    "PlayLooped",
    "PlayLoopedScheduled",
    "PlayScheduled",
    "Set",
    "SetAdd",
    "SetAddPointed",
    "SetAddShifted",
    "SetDivide",
    "SetDividePointed",
    "SetDivideShifted",
    "SetMod",
    "SetModPointed",
    "SetModShifted",
    "SetMultiply",
    "SetMultiplyPointed",
    "SetMultiplyShifted",
    "SetPointed",
    "SetPower",
    "SetPowerPointed",
    "SetPowerShifted",
    "SetRem",
    "SetRemPointed",
    "SetRemShifted",
    "SetShifted",
    "SetSubtract",
    "SetSubtractPointed",
    "SetSubtractShifted",
    "Spawn",
    "SpawnParticleEffect",
    "StopLooped",
    "StopLoopedScheduled",
    "StreamGetNextKey",
    "StreamGetPreviousKey",
    "StreamGetValue",
    "StreamHas",
    "Switch",
    "SwitchInteger",
    "SwitchIntegerWithDefault",
    "SwitchWithDefault",
    "TimeToScaledTime",
    "TimeToStartingScaledTime",
    "TimeToStartingTime",
    "TimeToTimeScale",
    "While",
];

fn is_vm_cut_function(name: &str) -> bool {
    // Preserve WatchVm's exact floating-point result bits for interpolation
    // and easing. Native Lerp differed on NaN payloads; keep the full family
    // on the oracle path rather than trusting only ordinary numeric inputs.
    VM_CUT_FUNCTIONS.contains(&name) || ease_parts(name).is_some()
}

fn ease_parts(name: &str) -> Option<(&'static str, &'static str)> {
    let suffix = name.strip_prefix("Ease")?;
    let (direction, family) =
        ["InOut", "OutIn", "In", "Out"]
            .into_iter()
            .find_map(|direction| {
                suffix
                    .strip_prefix(direction)
                    .map(|family| (direction, family))
            })?;
    let family = match family {
        "Sine" => "Sine",
        "Quad" => "Quad",
        "Cubic" => "Cubic",
        "Quart" => "Quart",
        "Quint" => "Quint",
        "Expo" => "Expo",
        "Circ" => "Circ",
        "Back" => "Back",
        "Elastic" => "Elastic",
        _ => return None,
    };
    Some((direction, family))
}

fn eligible_operation_count(nodes: &[EngineNode]) -> u64 {
    let mut states = vec![0; nodes.len()];
    nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            !is_numeric(node)
                && node
                    .func
                    .as_deref()
                    .is_some_and(|name| !is_vm_cut_function(name))
                && eligible(nodes, *index, &mut states, 0)
        })
        .count() as u64
}

#[derive(Clone, Copy)]
struct StaticVmMetrics {
    evaluations: usize,
    max_depth: usize,
}

fn static_vm_metrics(
    nodes: &[EngineNode],
    index: usize,
    active: &mut HashSet<usize>,
    depth: usize,
) -> Option<StaticVmMetrics> {
    if depth >= MAX_REGION_DEPTH || !active.insert(index) {
        return None;
    }
    let node = nodes.get(index)?;
    let result = if is_numeric(node) {
        Some(StaticVmMetrics {
            evaluations: 1,
            max_depth: depth,
        })
    } else {
        let name = node.func.as_deref()?;
        let all = child_indices(node, index).ok()?;
        let children = used_children(name, &all)
            .or_else(|_| {
                is_vm_cut_function(name)
                    .then_some(all)
                    .ok_or_else(|| anyhow::anyhow!("not a native node or VM cut"))
            })
            .ok()?;
        let mut evaluations = 1usize;
        let mut max_depth = depth;
        for child in children {
            let metrics = static_vm_metrics(nodes, child, active, depth + 1)?;
            evaluations = evaluations.checked_add(metrics.evaluations)?;
            max_depth = max_depth.max(metrics.max_depth);
            if evaluations > MAX_REGION_EVALUATIONS {
                active.remove(&index);
                return None;
            }
        }
        Some(StaticVmMetrics {
            evaluations,
            max_depth,
        })
    };
    active.remove(&index);
    result
}

fn vm_cut_metrics(
    nodes: &[EngineNode],
    index: usize,
    node: &EngineNode,
) -> Option<StaticVmMetrics> {
    if !node.func.as_deref().is_some_and(is_vm_cut_function) || nodes.get(index).is_none() {
        return None;
    }
    static_vm_metrics(nodes, index, &mut HashSet::new(), 0)
}

fn is_vm_cut(nodes: &[EngineNode], index: usize, node: &EngineNode) -> bool {
    vm_cut_metrics(nodes, index, node).is_some()
}

fn eligible(nodes: &[EngineNode], index: usize, states: &mut [u8], depth: usize) -> bool {
    if depth >= MAX_REGION_DEPTH {
        return false;
    }
    let Some(node) = nodes.get(index) else {
        return false;
    };
    if is_numeric(node) {
        return true;
    }
    if is_vm_cut(nodes, index, node) {
        return true;
    }
    match states[index] {
        1 => return false,
        2 => return true,
        3 => return false,
        _ => {}
    }
    states[index] = 1;
    let result = node
        .func
        .as_deref()
        .and_then(|name| {
            let children = child_indices(node, index).ok()?;
            let used = used_children(name, &children).ok()?;
            Some(
                used.into_iter()
                    .all(|child| eligible(nodes, child, states, depth + 1)),
            )
        })
        .unwrap_or(false);
    states[index] = if result { 2 } else { 3 };
    result
}

struct Codegen<'a> {
    nodes: &'a [EngineNode],
    active: HashSet<usize>,
    cut_nodes: Vec<usize>,
    cut_depths: Vec<usize>,
    evaluations: usize,
    compiled_evaluations: usize,
    max_depth: usize,
    function_counts: BTreeMap<String, u64>,
    compiled_nodes: HashSet<usize>,
}

impl<'a> Codegen<'a> {
    fn expression(&mut self, index: usize, function_depth: usize) -> Result<String> {
        self.evaluations += 1;
        self.max_depth = self.max_depth.max(function_depth);
        if self.evaluations > MAX_REGION_EVALUATIONS {
            bail!("pure region exceeds {MAX_REGION_EVALUATIONS} VM evaluations");
        }
        let node = self
            .nodes
            .get(index)
            .with_context(|| format!("Watch node index {index} is out of bounds"))?;
        if let Some(value) = node.value.as_ref().and_then(serde_json::Value::as_f64) {
            self.compiled_evaluations += 1;
            return Ok(format!("f64::from_bits(0x{:016x})", value.to_bits()));
        }
        if is_vm_cut(self.nodes, index, node) {
            // The operation, including any effect or lazy control behavior,
            // remains in WatchVm. Only its scalar result crosses the native
            // boundary, in VM argument-evaluation order.
            let metrics = vm_cut_metrics(self.nodes, index, node)
                .context("safe WatchVm cut lost its static evaluation metrics")?;
            self.evaluations = self
                .evaluations
                .saturating_add(metrics.evaluations.saturating_sub(1));
            if self.evaluations > MAX_REGION_EVALUATIONS {
                bail!("pure region exceeds {MAX_REGION_EVALUATIONS} VM evaluations");
            }
            let cut_depth = function_depth.saturating_add(metrics.max_depth);
            if cut_depth >= MAX_REGION_DEPTH {
                bail!("pure region exceeds the safe native nesting depth");
            }
            self.max_depth = self.max_depth.max(cut_depth);
            let slot = self.cut_nodes.len();
            self.cut_nodes.push(index);
            self.cut_depths.push(function_depth);
            return Ok(format!("inputs[{slot}]"));
        }
        if !self.active.insert(index) {
            bail!("cyclic Watch graph at node {index}");
        }
        let result = (|| {
            let name = node
                .func
                .as_deref()
                .with_context(|| format!("Watch node {index} has no numeric value or function"))?;
            if function_depth >= MAX_REGION_DEPTH {
                bail!("pure region exceeds the safe native nesting depth");
            }
            self.max_depth = self.max_depth.max(function_depth);
            self.compiled_evaluations += 1;
            *self.function_counts.entry(name.to_owned()).or_default() += 1;
            self.compiled_nodes.insert(index);
            let children = child_indices(node, index)?;
            let used = used_children(name, &children)?;
            let values = used
                .iter()
                .map(|child| self.expression(*child, function_depth + 1))
                .collect::<Result<Vec<_>>>()?;
            self.apply(name, &values)
                .with_context(|| format!("compiling Watch function {name} at node {index}"))
        })();
        self.active.remove(&index);
        result
    }

    fn apply(&self, name: &str, args: &[String]) -> Result<String> {
        let fold = |initial: Option<&str>, operator: &str| -> Result<String> {
            let first = args
                .first()
                .context("variadic operation requires an argument")?;
            let mut result =
                initial.map_or_else(|| first.clone(), |start| format!("({start} + {first})"));
            for value in &args[1..] {
                result = match operator {
                    "+" | "*" | "-" | "/" | "%" => format!("({result} {operator} {value})"),
                    "powf" => format!("({result}).powf({value})"),
                    "min" | "max" => format!("({result}).{operator}({value})"),
                    _ => unreachable!(),
                };
            }
            Ok(result)
        };
        match name {
            "Add" => fold(Some("0.0_f64"), "+"),
            "Multiply" => {
                let mut result = format!("(1.0_f64 * {})", args[0]);
                for value in &args[1..] {
                    result = format!("({result} * {value})");
                }
                Ok(result)
            }
            "Min" => fold(None, "min"),
            "Max" => fold(None, "max"),
            "Rem" => fold(None, "%"),
            "Subtract" => fold(None, "-"),
            "Divide" => fold(None, "/"),
            "Power" => fold(None, "powf"),
            "Equal" | "NotEqual" | "Greater" | "GreaterOr" | "Less" | "LessOr" => {
                let operator = match name {
                    "Equal" => "==",
                    "NotEqual" => "!=",
                    "Greater" => ">",
                    "GreaterOr" => ">=",
                    "Less" => "<",
                    "LessOr" => "<=",
                    _ => unreachable!(),
                };
                Ok(format!(
                    "if {} {operator} {} {{ 1.0_f64 }} else {{ 0.0_f64 }}",
                    args[0], args[1]
                ))
            }
            "Arctan2" => Ok(format!("({}).atan2({})", args[0], args[1])),
            "Abs" => Ok(format!("({}).abs()", args[0])),
            "Arctan" => Ok(format!("({}).atan()", args[0])),
            "Ceil" => Ok(format!("({}).ceil()", args[0])),
            "Cos" => Ok(format!("({}).cos()", args[0])),
            "Floor" => Ok(format!("({}).floor()", args[0])),
            "Log" => Ok(format!("({}).ln()", args[0])),
            "Negate" => Ok(format!("-({})", args[0])),
            "Not" => Ok(format!(
                "if ({}) == 0.0 {{ 1.0_f64 }} else {{ 0.0_f64 }}",
                args[0]
            )),
            "Round" => Ok(format!("({}).round()", args[0])),
            "Sin" => Ok(format!("({}).sin()", args[0])),
            "Trunc" => Ok(format!("({}).trunc()", args[0])),
            "Frac" => Ok(format!(
                "{{ let value = {}; value - value.trunc() }}",
                args[0]
            )),
            "Sign" => Ok(format!("({}).signum()", args[0])),
            "Radian" => Ok(format!("({}).to_radians()", args[0])),
            "Degree" => Ok(format!("({}).to_degrees()", args[0])),
            "Arcsin" => Ok(format!("({}).asin()", args[0])),
            "Arccos" => Ok(format!("({}).acos()", args[0])),
            "Tan" => Ok(format!("({}).tan()", args[0])),
            "Cosh" => Ok(format!("({}).cosh()", args[0])),
            "Sinh" => Ok(format!("({}).sinh()", args[0])),
            "Tanh" => Ok(format!("({}).tanh()", args[0])),
            "Clamp" => Ok(format!(
                "{{ let v0 = {}; let v1 = {}; let v2 = {}; v0.clamp(v1.min(v2), v1.max(v2)) }}",
                args[0], args[1], args[2]
            )),
            "Lerp" | "LerpClamped" => Ok(format!(
                "{{ let from = {}; let to = {}; let mut t = {}; {} from + (to - from) * t }}",
                args[0],
                args[1],
                args[2],
                if name == "LerpClamped" {
                    "t = t.clamp(0.0, 1.0);"
                } else {
                    ""
                },
            )),
            "Unlerp" | "UnlerpClamped" => Ok(format!(
                "{{ let min = {}; let max = {}; let value = {}; let t = (value - min) / (max - min); {} }}",
                args[0],
                args[1],
                args[2],
                if name == "UnlerpClamped" {
                    "t.clamp(0.0, 1.0)"
                } else {
                    "t"
                }
            )),
            "Remap" | "RemapClamped" => Ok(format!(
                "{{ let from_min = {}; let from_max = {}; let to_min = {}; let to_max = {}; let value = {}; let t = (value - from_min) / (from_max - from_min); let t = {}; to_min + (to_max - to_min) * t }}",
                args[0],
                args[1],
                args[2],
                args[3],
                args[4],
                if name == "RemapClamped" {
                    "t.clamp(0.0, 1.0)"
                } else {
                    "t"
                }
            )),
            _ if ease_parts(name).is_some() => ease_expression(name, &args[0])
                .context("building native easing expression"),
            _ => bail!("{name} is not a Sono-GCC pure scalar operation"),
        }
    }
}

fn ease_expression(name: &str, value: &str) -> Option<String> {
    let (direction, family) = ease_parts(name)?;
    let input = "x";
    let ease_in = match family {
        "Sine" => format!("1.0 - ({input} * std::f64::consts::FRAC_PI_2).cos()"),
        "Quad" => format!("{input} * {input}"),
        "Cubic" => format!("{input} * {input} * {input}"),
        "Quart" => format!("{input} * {input} * {input} * {input}"),
        "Quint" => format!("{input} * {input} * {input} * {input} * {input}"),
        "Expo" => format!("if {input} == 0.0 {{ 0.0 }} else {{ 2.0_f64.powf(10.0 * {input} - 10.0) }}"),
        "Circ" => format!("1.0 - (1.0 - {input} * {input}).sqrt()"),
        "Back" => format!("{{ const C1: f64 = 1.70158; const C3: f64 = C1 + 1.0; C3 * {input} * {input} * {input} - C1 * {input} * {input} }}"),
        "Elastic" => format!("if {input} == 0.0 {{ 0.0 }} else if {input} == 1.0 {{ 1.0 }} else {{ -(2.0_f64).powf(10.0 * {input} - 10.0) * (({input} * 10.0 - 10.75) * (2.0 * std::f64::consts::PI / 3.0)).sin() }}"),
        _ => return None,
    };
    let ease_out = match family {
        "Sine" => format!("({input} * std::f64::consts::FRAC_PI_2).sin()"),
        "Quad" => format!("1.0 - (1.0 - {input}) * (1.0 - {input})"),
        "Cubic" => format!("1.0 - (1.0 - {input}).powi(3)"),
        "Quart" => format!("1.0 - (1.0 - {input}).powi(4)"),
        "Quint" => format!("1.0 - (1.0 - {input}).powi(5)"),
        "Expo" => format!("if {input} == 1.0 {{ 1.0 }} else {{ 1.0 - (2.0_f64).powf(-10.0 * {input}) }}"),
        "Circ" => format!("(1.0 - ({input} - 1.0) * ({input} - 1.0)).sqrt()"),
        "Back" => format!("{{ const C1: f64 = 1.70158; const C3: f64 = C1 + 1.0; 1.0 + C3 * ({input} - 1.0).powi(3) + C1 * ({input} - 1.0).powi(2) }}"),
        "Elastic" => format!("if {input} == 0.0 {{ 0.0 }} else if {input} == 1.0 {{ 1.0 }} else {{ (2.0_f64).powf(-10.0 * {input}) * (({input} * 10.0 - 0.75) * (2.0 * std::f64::consts::PI / 3.0)).sin() + 1.0 }}"),
        _ => return None,
    };
    let direction = match direction {
        "In" => format!("ease_in(value)"),
        "Out" => format!("ease_out(value)"),
        "InOut" => "if value < 0.5 { ease_in(value * 2.0) / 2.0 } else { ease_out(value * 2.0 - 1.0) / 2.0 + 0.5 }".to_owned(),
        "OutIn" => "if value < 0.5 { ease_out(value * 2.0) / 2.0 } else { ease_in(value * 2.0 - 1.0) / 2.0 + 0.5 }".to_owned(),
        _ => return None,
    };
    Some(format!(
        "{{ let value = {value}; let ease_in = |x: f64| -> f64 {{ {ease_in} }}; let ease_out = |x: f64| -> f64 {{ {ease_out} }}; {direction} }}"
    ))
}

fn compile_units(nodes: &[EngineNode]) -> Result<Vec<CompileUnit>> {
    let mut states = vec![0; nodes.len()];
    let candidates = (0..nodes.len())
        .map(|index| eligible(nodes, index, &mut states, 0))
        .collect::<Vec<_>>();
    let mut covered = vec![false; nodes.len()];
    for (index, node) in nodes.iter().enumerate() {
        if !candidates[index] || is_numeric(node) || is_vm_cut(nodes, index, node) {
            continue;
        }
        let Some(name) = node.func.as_deref() else {
            continue;
        };
        let children = child_indices(node, index)?;
        for child in used_children(name, &children)? {
            if candidates.get(child).copied().unwrap_or(false)
                && !is_numeric(&nodes[child])
                && !is_vm_cut(nodes, child, &nodes[child])
            {
                covered[child] = true;
            }
        }
    }
    let roots = (0..nodes.len())
        .filter(|&index| {
            candidates[index]
                && !covered[index]
                && !is_numeric(&nodes[index])
                && !is_vm_cut(nodes, index, &nodes[index])
        })
        .collect::<Vec<_>>();
    let units: Vec<CompileUnit> = roots
        .into_iter()
        .filter_map(|root| compile_unit(nodes, root))
        .collect();
    Ok(units)
}

fn dispatchable_units(units: Vec<CompileUnit>) -> Vec<CompileUnit> {
    units
        .into_iter()
        .filter(|unit| unit.compiled_nodes.len() >= MIN_NATIVE_REGION_NODES)
        .collect()
}

fn compile_unit(nodes: &[EngineNode], root: usize) -> Option<CompileUnit> {
    let mut codegen = Codegen {
        nodes,
        active: HashSet::new(),
        cut_nodes: Vec::new(),
        cut_depths: Vec::new(),
        evaluations: 0,
        compiled_evaluations: 0,
        max_depth: 0,
        function_counts: BTreeMap::new(),
        compiled_nodes: HashSet::new(),
    };
    let Ok(expression) = codegen.expression(root, 0) else {
        return None;
    };
    if codegen.evaluations > MAX_REGION_EVALUATIONS || codegen.max_depth >= MAX_REGION_DEPTH {
        return None;
    }
    let mut compiled_nodes = codegen.compiled_nodes.into_iter().collect::<Vec<_>>();
    compiled_nodes.sort_unstable();
    let function_dispatch_count = codegen.function_counts.values().sum();
    let function_counts = compact_function_counts(&codegen.function_counts);
    Some(CompileUnit {
        root,
        expression,
        cut_nodes: codegen.cut_nodes,
        cut_depths: codegen.cut_depths,
        metrics: RegionMetrics {
            evaluations: codegen.evaluations,
            compiled_evaluations: codegen.compiled_evaluations,
            max_depth: codegen.max_depth,
        },
        function_counts,
        function_dispatch_count,
        compiled_nodes,
    })
}

#[cfg(windows)]
struct NativeLibrary {
    handle: usize,
    directory: std::path::PathBuf,
    library_path: std::path::PathBuf,
    remove_directory: bool,
}

#[cfg(windows)]
impl Drop for NativeLibrary {
    fn drop(&mut self) {
        unsafe { FreeLibrary(self.handle as *mut std::ffi::c_void) };
        if self.remove_directory {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn LoadLibraryW(filename: *const u16) -> *mut std::ffi::c_void;
    fn GetProcAddress(
        module: *mut std::ffi::c_void,
        name: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn FreeLibrary(module: *mut std::ffi::c_void) -> i32;
}

#[cfg(windows)]
fn compile_windows(
    nodes: &[EngineNode],
    started: Instant,
) -> Result<(SonoGccProgram, Duration, Duration, Duration)> {
    use std::{
        process::Command,
        sync::atomic::{AtomicU64, Ordering},
    };
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    let region_codegen_started = Instant::now();
    let (units, source) = compile_source(nodes)?;
    let region_codegen = region_codegen_started.elapsed();

    let directory = std::env::temp_dir().join(format!(
        "sono-renderer-gcc-{}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).context("creating Sono-GCC temporary directory")?;
    let source_path = directory.join("compiled_regions.rs");
    let library_path = directory.join("compiled_regions.dll");
    std::fs::write(&source_path, source).context("writing generated Sono-GCC source")?;
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let rustc_started = Instant::now();
    let output = crate::export_control::hide_child_window(&mut Command::new(rustc))
        .arg("--edition=2021")
        .arg("--crate-type=cdylib")
        .arg("-Copt-level=3")
        .arg("-Awarnings")
        .arg(&source_path)
        .arg("-o")
        .arg(&library_path)
        .output()
        .context("starting rustc for Sono-GCC regions")?;
    let rustc_compile = rustc_started.elapsed();
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&directory);
        bail!(
            "rustc failed to compile Sono-GCC regions: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let (program, _) = load_windows_library(nodes.len(), units, &library_path, &directory, true)?;
    Ok((program, started.elapsed(), region_codegen, rustc_compile))
}

#[cfg(windows)]
fn compile_source(nodes: &[EngineNode]) -> Result<(Vec<CompileUnit>, String)> {
    let units = dispatchable_units(compile_units(nodes)?);
    if units.is_empty() {
        bail!("no compilable Watch regions were found");
    }
    let mut source = String::from("#![allow(unused_variables, unused_parens)]\n");
    for (function_index, unit) in units.iter().enumerate() {
        source.push_str(&format!(
            "#[no_mangle]\npub unsafe extern \"C\" fn sono_run_{function_index}(input_ptr: *const f64, input_len: usize) -> f64 {{\n\
             if input_len != {} || (input_len != 0 && input_ptr.is_null()) {{ return f64::NAN; }}\n\
             let inputs = std::slice::from_raw_parts(input_ptr, input_len);\n\
             {}\n}}\n",
            unit.cut_nodes.len(), unit.expression
        ));
        if source.len() > MAX_NATIVE_SOURCE_BYTES {
            bail!(
                "Sono-GCC source size {} bytes exceeds the {} byte safety limit",
                source.len(),
                MAX_NATIVE_SOURCE_BYTES
            );
        }
    }
    Ok((units, source))
}

#[cfg(windows)]
fn load_windows_library(
    graph_nodes: usize,
    units: Vec<CompileUnit>,
    library_path: &std::path::Path,
    directory: &std::path::Path,
    remove_directory: bool,
) -> Result<(SonoGccProgram, Duration)> {
    use std::{ffi::CString, os::windows::ffi::OsStrExt};

    let native_load_started = Instant::now();
    let wide_path = library_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let handle = unsafe { LoadLibraryW(wide_path.as_ptr()) };
    if handle.is_null() {
        if remove_directory {
            let _ = std::fs::remove_dir_all(directory);
        }
        bail!(
            "Windows could not load the Sono-GCC DLL at {}",
            library_path.display()
        );
    }
    let mut region_by_node = vec![u32::MAX; graph_nodes];
    let mut regions = Vec::with_capacity(units.len());
    for (function_index, unit) in units.into_iter().enumerate() {
        let Ok(region_index) = u32::try_from(regions.len()) else {
            unsafe { FreeLibrary(handle) };
            if remove_directory {
                let _ = std::fs::remove_dir_all(directory);
            }
            bail!("Sono-GCC compiled-region index exceeds its 32-bit dispatch table");
        };
        if region_index == u32::MAX || unit.root >= graph_nodes {
            unsafe { FreeLibrary(handle) };
            if remove_directory {
                let _ = std::fs::remove_dir_all(directory);
            }
            bail!("Sono-GCC compiled-region root is outside its dispatch table");
        }
        let symbol = CString::new(format!("sono_run_{function_index}"))?;
        let address = unsafe { GetProcAddress(handle, symbol.as_ptr()) };
        if address.is_null() {
            unsafe { FreeLibrary(handle) };
            if remove_directory {
                let _ = std::fs::remove_dir_all(directory);
            }
            bail!("Sono-GCC DLL is missing {symbol:?}");
        }
        let run = unsafe { std::mem::transmute::<*mut std::ffi::c_void, NativeRun>(address) };
        regions.push(std::sync::Arc::new(CompiledRegion {
            root: unit.root,
            run,
            cut_nodes: unit.cut_nodes,
            cut_depths: unit.cut_depths,
            metrics: unit.metrics,
            function_counts: unit.function_counts,
            function_dispatch_count: unit.function_dispatch_count,
            compiled_nodes: unit.compiled_nodes,
        }));
        region_by_node[unit.root] = region_index;
    }
    let load_time = native_load_started.elapsed();
    let program = SonoGccProgram {
        region_by_node,
        regions,
        operation_counts: std::sync::OnceLock::new(),
        native_load_time: load_time,
        _library: NativeLibrary {
            handle: handle as usize,
            directory: directory.to_path_buf(),
            library_path: library_path.to_path_buf(),
            remove_directory,
        },
        _cache_lease: None,
    };
    Ok((program, load_time))
}

#[cfg(windows)]
#[derive(Debug, Serialize, Deserialize)]
struct CacheManifest {
    format: String,
    schema: u32,
    cache_key: String,
    renderer_version: String,
    created_unix_ms: u128,
    compiler_backend: String,
    runtime_abi: String,
    rustc_identity: String,
    target: String,
    flags: String,
    graph_sha1: String,
    graph_len: usize,
    library_name: String,
    library_sha1: String,
    library_len: u64,
    metadata_name: String,
    metadata_sha1: String,
    metadata_len: usize,
    region_count: u64,
    original_compile_ms: u128,
}

#[cfg(windows)]
#[derive(Serialize, Deserialize)]
struct CachedRegionMetadata {
    root: usize,
    cut_nodes: Vec<usize>,
    cut_depths: Vec<usize>,
    metrics: RegionMetrics,
    function_counts: Vec<(u8, u64)>,
    function_dispatch_count: u64,
    compiled_nodes: Vec<usize>,
}

#[cfg(windows)]
#[derive(Serialize, Deserialize)]
struct CachedProgramMetadata {
    schema: u32,
    graph_sha1: String,
    #[serde(default)]
    compiled_operations: Option<u64>,
    #[serde(default)]
    eligible_operations: Option<u64>,
    regions: Vec<CachedRegionMetadata>,
}

#[cfg(windows)]
struct CacheIdentity {
    graph_bytes: Vec<u8>,
    digest: String,
    graph_sha1: String,
    rustc_identity: String,
    target: String,
    directory: std::path::PathBuf,
    identity_time: Duration,
}

#[cfg(windows)]
const CACHE_ENTRY_FORMAT: &str = "sono-renderer-cache-entry";
#[cfg(windows)]
const CACHE_MANIFEST_SCHEMA: u32 = 4;
#[cfg(windows)]
const MAX_CACHE_MANIFEST_BYTES: u64 = 1024 * 1024;
#[cfg(windows)]
const MAX_CACHE_PAYLOAD_BYTES: u64 = 256 * 1024 * 1024;

fn cache_root() -> std::path::PathBuf {
    #[cfg(all(windows, test))]
    {
        test_cache_base()
            .join("appdata")
            .join("Sono-Renderer")
            .join("cache")
            .join("sono-gcc-v2")
    }
    #[cfg(not(all(windows, test)))]
    {
        crate::store_paths::cache_root().join("sono-gcc-v2")
    }
}

fn fallback_cache_root() -> std::path::PathBuf {
    #[cfg(all(windows, test))]
    {
        test_cache_base()
            .join("temp")
            .join("Sono-Renderer")
            .join("cache")
            .join("sono-gcc-v2")
    }
    #[cfg(not(all(windows, test)))]
    {
        std::env::temp_dir()
            .join("Sono-Renderer")
            .join("cache")
            .join("sono-gcc-v2")
    }
}

#[cfg(all(windows, test))]
static TEST_CACHE_ROOT: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();

#[cfg(all(windows, test))]
fn test_cache_base() -> std::path::PathBuf {
    TEST_CACHE_ROOT
        .get_or_init(|| tempfile::tempdir().expect("creating isolated Sono-GCC test cache"))
        .path()
        .to_path_buf()
}

pub(crate) fn persistent_cache_roots() -> Vec<(String, std::path::PathBuf)> {
    #[cfg(windows)]
    {
        let mut roots = vec![("appdata".to_owned(), cache_root())];
        let fallback = fallback_cache_root();
        if roots.iter().all(|(_, root)| root != &fallback) {
            roots.push(("temp".to_owned(), fallback));
        }
        roots
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
pub(crate) fn load_cached_watch_graph(store: &str, key: &str) -> Result<Vec<EngineNode>> {
    use anyhow::Context;
    crate::cache_manager::validate_key(key)?;
    let root = persistent_cache_roots()
        .into_iter()
        .find_map(|(candidate, root)| (candidate == store).then_some(root))
        .context("persistent cache store is unavailable")?;
    let directory = root.join(key);
    let Some(lease) = crate::cache_manager::entry_lock_if_present(
        &root,
        key,
        crate::cache_manager::EntryLockMode::Shared,
    )?
    else {
        bail!("cache entry {key} was not found");
    };
    let result = (|| {
        match std::fs::symlink_metadata(&directory) {
            Ok(meta) if meta.file_type().is_dir() && !meta.file_type().is_symlink() => {}
            _ => bail!("cache entry directory is missing or unsafe"),
        }
        let manifest_path = directory.join("manifest.json");
        let manifest_info =
            std::fs::symlink_metadata(&manifest_path).context("cache manifest is missing")?;
        if !manifest_info.file_type().is_file()
            || manifest_info.file_type().is_symlink()
            || manifest_info.len() > MAX_CACHE_MANIFEST_BYTES
        {
            bail!("cache manifest is unsafe or too large");
        }
        let manifest: CacheManifest = serde_json::from_slice(&std::fs::read(&manifest_path)?)
            .context("parsing cache manifest")?;
        if manifest.schema != CACHE_MANIFEST_SCHEMA || manifest.cache_key != key {
            bail!("cache graph cannot be rebuilt from an incompatible manifest");
        }
        let graph_path = directory.join("graph.json");
        let graph_info =
            std::fs::symlink_metadata(&graph_path).context("cached graph payload is missing")?;
        if !graph_info.file_type().is_file()
            || graph_info.file_type().is_symlink()
            || graph_info.len() > MAX_CACHE_PAYLOAD_BYTES
            || graph_info.len() != manifest.graph_len as u64
        {
            bail!("cached graph payload is unsafe or has an invalid length");
        }
        let bytes = std::fs::read(&graph_path).context("reading cached Watch graph")?;
        if sha1_hex(&bytes) != manifest.graph_sha1
            || graph_key_from_manifest(&manifest, &bytes) != key
        {
            bail!("cached graph identity or checksum is invalid");
        }
        serde_json::from_slice(&bytes).context("decoding cached Watch graph")
    })();
    lease.unlock()?;
    result
}

#[cfg(not(windows))]
pub(crate) fn load_cached_watch_graph(_store: &str, _key: &str) -> Result<Vec<EngineNode>> {
    bail!("persistent Sono-GCC cache is available only on Windows")
}

/// Remove Sono-GCC's persistent compiled-code cache. The cache is fully
/// reproducible and will be rebuilt on the next precompile/render.
#[cfg(windows)]
pub fn clear_persistent_cache() -> anyhow::Result<bool> {
    let report = crate::cache_manager::clear(false)?;
    if report.skipped_active != 0 {
        bail!(
            "left {} active Sono-GCC cache entr{} untouched",
            report.skipped_active,
            if report.skipped_active == 1 {
                "y"
            } else {
                "ies"
            }
        );
    }
    Ok(report.removed_entries != 0)
}

#[cfg(not(windows))]
pub fn clear_persistent_cache() -> anyhow::Result<bool> {
    anyhow::bail!("Sono-GCC persistent cache is available only on Windows")
}

#[cfg(windows)]
pub(crate) fn inspect_persistent_cache() -> Result<Vec<crate::cache_manager::CacheEntry>> {
    let mut entries = Vec::new();
    for (store, root) in persistent_cache_roots() {
        match std::fs::symlink_metadata(&root) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("reading cache root {}", root.display()))
            }
        }
        for item in std::fs::read_dir(&root)
            .with_context(|| format!("scanning cache root {}", root.display()))?
        {
            let item = item?;
            let kind = item.file_type()?;
            let key = item.file_name().to_string_lossy().into_owned();
            if !kind.is_dir()
                || kind.is_symlink()
                || key.len() != 40
                || !key.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                continue;
            }
            if let Some(entry) = inspect_cache_entry(&store, &root, &key, false)? {
                entries.push(entry);
            }
        }
    }
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entries)
}

#[cfg(not(windows))]
pub(crate) fn inspect_persistent_cache() -> Result<Vec<crate::cache_manager::CacheEntry>> {
    Ok(Vec::new())
}

#[cfg(windows)]
pub(crate) fn validate_persistent_cache() -> Result<Vec<crate::cache_manager::CacheEntry>> {
    let mut entries = Vec::new();
    for (store, root) in persistent_cache_roots() {
        let Ok(root_info) = std::fs::symlink_metadata(&root) else {
            continue;
        };
        if !root_info.file_type().is_dir() || root_info.file_type().is_symlink() {
            continue;
        }
        for item in std::fs::read_dir(&root)? {
            let item = item?;
            let kind = item.file_type()?;
            let key = item.file_name().to_string_lossy().into_owned();
            if kind.is_dir()
                && !kind.is_symlink()
                && key.len() == 40
                && key.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                if let Some(entry) = inspect_cache_entry(&store, &root, &key, true)? {
                    entries.push(entry);
                }
            }
        }
    }
    entries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entries)
}

#[cfg(windows)]
pub(crate) fn validate_persistent_cache_entry(
    store: &str,
    key: &str,
) -> Result<Option<crate::cache_manager::CacheEntry>> {
    let Some((_, root)) = persistent_cache_roots()
        .into_iter()
        .find(|(candidate, _)| candidate == store)
    else {
        return Ok(None);
    };
    inspect_cache_entry(store, &root, key, true)
}

#[cfg(not(windows))]
pub(crate) fn validate_persistent_cache() -> Result<Vec<crate::cache_manager::CacheEntry>> {
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub(crate) fn validate_persistent_cache_entry(
    _store: &str,
    _key: &str,
) -> Result<Option<crate::cache_manager::CacheEntry>> {
    Ok(None)
}

#[cfg(windows)]
fn inspect_cache_entry(
    store: &str,
    root: &std::path::Path,
    key: &str,
    validate: bool,
) -> Result<Option<crate::cache_manager::CacheEntry>> {
    use crate::cache_manager::{CacheEntry, CacheEntryStatus};

    let directory = root.join(key);
    let _lock = crate::cache_manager::try_entry_inspection_lock(root, key)?;
    let dir_info = match std::fs::symlink_metadata(&directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            metadata
        }
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading cache entry {}", directory.display()))
        }
    };
    let size_bytes = crate::cache_manager::directory_size(&directory)?;
    let last_used_unix_ms = std::fs::metadata(directory.join("last-used"))
        .ok()
        .and_then(|metadata| metadata.modified().ok())
        .and_then(system_time_unix_ms);
    let mut status = CacheEntryStatus::Corrupt;
    let mut diagnostic = None;
    let mut schema = None;
    let mut target = None;
    let mut created_unix_ms = dir_info.created().ok().and_then(system_time_unix_ms);

    let validation = if validate {
        inspect_cache_payload(&directory, key)
    } else {
        inspect_cache_manifest(&directory, key).and_then(|manifest| {
            let compatible = cache_manifest_is_compatible(&manifest)?;
            Ok((manifest, compatible))
        })
    };
    match validation {
        Ok((manifest, compatible)) => {
            schema = Some(manifest.schema);
            target = Some(manifest.target.clone());
            created_unix_ms = Some(manifest.created_unix_ms);
            if compatible {
                status = if validate {
                    CacheEntryStatus::Valid
                } else {
                    CacheEntryStatus::Unverified
                };
                if !validate {
                    diagnostic =
                        Some("integrity hashes have not been checked; run `cache validate`".into());
                }
            } else {
                status = CacheEntryStatus::Incompatible;
                diagnostic = Some("entry is valid but targets a different renderer/compiler/runtime/target identity".into());
            }
        }
        Err(error) => {
            diagnostic = Some(format!("{error:#}"));
            if diagnostic
                .as_deref()
                .is_some_and(|message| message.contains("older or unknown schema"))
            {
                status = CacheEntryStatus::Incompatible;
            }
        }
    }
    Ok(Some(CacheEntry {
        id: format!("{store}:{key}"),
        category: "Sono-GCC compiled Watch graph",
        associated_data: format!("Watch graph {}", &key[..12]),
        size_bytes,
        created_unix_ms,
        last_used_unix_ms,
        schema,
        target,
        status,
        diagnostic,
    }))
}

#[cfg(windows)]
fn inspect_cache_payload(directory: &std::path::Path, key: &str) -> Result<(CacheManifest, bool)> {
    use anyhow::Context;
    let manifest = inspect_cache_manifest(directory, key)?;
    let graph_path = directory.join("graph.json");
    let graph_len = std::fs::symlink_metadata(&graph_path).context("graph payload is missing")?;
    if graph_len.len() > MAX_CACHE_PAYLOAD_BYTES || graph_len.len() != manifest.graph_len as u64 {
        bail!("graph payload length does not match manifest");
    }
    let graph_bytes = std::fs::read(&graph_path).context("reading graph payload")?;
    if sha1_hex(&graph_bytes) != manifest.graph_sha1
        || graph_key_from_manifest(&manifest, &graph_bytes) != key
    {
        bail!("graph payload checksum or cache identity does not match");
    }
    let library_path = directory.join(&manifest.library_name);
    let library_info =
        std::fs::symlink_metadata(&library_path).context("compiled library is missing")?;
    if library_info.len() > MAX_CACHE_PAYLOAD_BYTES || library_info.len() != manifest.library_len {
        bail!("compiled library length does not match manifest");
    }
    let library_bytes = std::fs::read(&library_path).context("reading compiled library")?;
    if sha1_hex(&library_bytes) != manifest.library_sha1 {
        bail!("compiled library checksum does not match manifest");
    }
    let metadata_path = directory.join(&manifest.metadata_name);
    let metadata_info =
        std::fs::symlink_metadata(&metadata_path).context("region metadata is missing")?;
    if metadata_info.len() > MAX_CACHE_PAYLOAD_BYTES
        || metadata_info.len() != manifest.metadata_len as u64
    {
        bail!("region metadata length does not match manifest");
    }
    let metadata_bytes = std::fs::read(&metadata_path).context("reading region metadata")?;
    if sha1_hex(&metadata_bytes) != manifest.metadata_sha1 {
        bail!("region metadata checksum does not match manifest");
    }
    let cached_metadata: CachedProgramMetadata =
        serde_json::from_slice(&metadata_bytes).context("parsing region metadata")?;
    let nodes: Vec<EngineNode> =
        serde_json::from_slice(&graph_bytes).context("parsing cached Watch graph")?;
    validate_cached_metadata(&cached_metadata, nodes.len(), &manifest)?;
    let compatible = cache_manifest_is_compatible(&manifest)?;
    Ok((manifest, compatible))
}

#[cfg(windows)]
fn inspect_cache_manifest(directory: &std::path::Path, key: &str) -> Result<CacheManifest> {
    use anyhow::Context;
    let manifest_path = directory.join("manifest.json");
    let manifest_info = std::fs::symlink_metadata(&manifest_path).context("manifest is missing")?;
    if !manifest_info.file_type().is_file()
        || manifest_info.file_type().is_symlink()
        || manifest_info.len() > MAX_CACHE_MANIFEST_BYTES
    {
        bail!("cache manifest is not a regular file within the size limit");
    }
    let bytes = std::fs::read(&manifest_path).context("reading cache manifest")?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).context("parsing manifest JSON")?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    if schema != u64::from(CACHE_MANIFEST_SCHEMA) {
        bail!("cache manifest is from an older or unknown schema ({schema})");
    }
    let manifest: CacheManifest =
        serde_json::from_value(value).context("decoding cache manifest")?;
    if manifest.format != CACHE_ENTRY_FORMAT
        || manifest.cache_key != key
        || !is_single_file_name(&manifest.library_name)
        || !manifest.library_name.ends_with(".dll")
        || !is_single_file_name(&manifest.metadata_name)
        || !manifest.metadata_name.ends_with(".json")
        || manifest.graph_len as u64 > MAX_CACHE_PAYLOAD_BYTES
        || manifest.library_len > MAX_CACHE_PAYLOAD_BYTES
        || manifest.metadata_len as u64 > MAX_CACHE_PAYLOAD_BYTES
    {
        bail!("manifest identity, artifact path, or payload size is invalid");
    }
    let required = [
        ("graph.json", manifest.graph_len as u64),
        (&manifest.library_name, manifest.library_len),
        (&manifest.metadata_name, manifest.metadata_len as u64),
    ];
    for (name, expected_len) in required {
        let info = std::fs::symlink_metadata(directory.join(name))
            .with_context(|| format!("required cache payload {name:?} is missing"))?;
        if !info.file_type().is_file()
            || info.file_type().is_symlink()
            || info.len() != expected_len
        {
            bail!("cache payload {name:?} has an unsafe type or mismatched length");
        }
    }
    Ok(manifest)
}

#[cfg(windows)]
fn cache_manifest_is_compatible(manifest: &CacheManifest) -> Result<bool> {
    let (rustc_identity, target) = current_rustc_identity()?;
    Ok(manifest.compiler_backend == COMPILER_BACKEND_VERSION
        && manifest.runtime_abi == COMPILED_RUNTIME_ABI_VERSION
        && manifest.renderer_version == env!("CARGO_PKG_VERSION")
        && manifest.rustc_identity == rustc_identity
        && manifest.target == target
        && manifest.flags == COMPILER_FLAGS_ID)
}

#[cfg(windows)]
fn is_single_file_name(name: &str) -> bool {
    let path = std::path::Path::new(name);
    path.file_name().is_some_and(|file| file == name)
        && !name.is_empty()
        && !name.contains(['/', '\\', ':'])
}

#[cfg(windows)]
fn graph_key_from_manifest(manifest: &CacheManifest, graph: &[u8]) -> String {
    let header = format!(
        "{}\0{}\0{}\0{}\0{}\0{}\0",
        manifest.compiler_backend,
        manifest.runtime_abi,
        manifest.renderer_version,
        manifest.flags,
        manifest.target,
        manifest.rustc_identity,
    );
    let mut digest = Sha1::new();
    digest.update(header.as_bytes());
    digest.update(graph);
    hex::encode(digest.finalize())
}

#[cfg(windows)]
fn validate_cached_metadata(
    metadata: &CachedProgramMetadata,
    node_count: usize,
    manifest: &CacheManifest,
) -> Result<()> {
    if metadata.schema != 1
        || metadata.graph_sha1 != manifest.graph_sha1
        || metadata.regions.len() != manifest.region_count as usize
        || metadata.regions.is_empty()
        || metadata
            .regions
            .windows(2)
            .any(|pair| pair[0].root >= pair[1].root)
        || metadata.regions.iter().any(|region| {
            region.root >= node_count
                || region.cut_nodes.len() != region.cut_depths.len()
                || region.cut_nodes.iter().any(|&index| index >= node_count)
                || region
                    .cut_depths
                    .iter()
                    .any(|&depth| depth >= MAX_REGION_DEPTH)
                || region.metrics.evaluations == 0
                || region.metrics.evaluations > MAX_REGION_EVALUATIONS
                || region.metrics.compiled_evaluations > region.metrics.evaluations
                || region.metrics.max_depth >= MAX_REGION_DEPTH
                || region.compiled_nodes.is_empty()
                || region
                    .compiled_nodes
                    .iter()
                    .any(|&index| index >= node_count)
                || region.function_counts.iter().any(|(index, count)| {
                    usize::from(*index) >= NATIVE_FUNCTION_COUNT || *count == 0
                })
                || region
                    .function_counts
                    .iter()
                    .map(|(_, count)| count)
                    .sum::<u64>()
                    != region.function_dispatch_count
        })
    {
        bail!("cached region metadata failed structural validation");
    }
    match (metadata.compiled_operations, metadata.eligible_operations) {
        (Some(compiled), Some(eligible))
            if compiled <= eligible && eligible <= node_count as u64 => {}
        (None, None) => {}
        _ => bail!("cached operation summary is invalid"),
    }
    Ok(())
}

#[cfg(windows)]
fn system_time_unix_ms(time: std::time::SystemTime) -> Option<u128> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis())
}

#[cfg(windows)]
fn touch_cache_last_used(directory: &std::path::Path) {
    let path = directory.join("last-used");
    if let Ok(file) = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
    {
        let _ =
            file.set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()));
    }
}

#[cfg(windows)]
fn sha1_hex(bytes: &[u8]) -> String {
    hex::encode(Sha1::digest(bytes))
}

#[cfg(windows)]
fn cache_identity(nodes: &[EngineNode]) -> Result<CacheIdentity> {
    let identity_started = Instant::now();
    let graph_bytes = serde_json::to_vec(nodes).context("serializing Watch graph identity")?;
    let (rustc_identity, target) = current_rustc_identity()?;
    let identity_header = format!(
        "{COMPILER_BACKEND_VERSION}\0{COMPILED_RUNTIME_ABI_VERSION}\0{}\0{COMPILER_FLAGS_ID}\0{target}\0{rustc_identity}\0",
        env!("CARGO_PKG_VERSION")
    );
    let mut identity_hasher = Sha1::new();
    identity_hasher.update(identity_header.as_bytes());
    identity_hasher.update(&graph_bytes);
    let digest = hex::encode(identity_hasher.finalize());
    let graph_sha1 = sha1_hex(&graph_bytes);
    Ok(CacheIdentity {
        graph_bytes,
        directory: cache_root().join(&digest),
        digest,
        graph_sha1,
        rustc_identity,
        target,
        identity_time: identity_started.elapsed(),
    })
}

#[cfg(windows)]
fn current_rustc_identity() -> Result<(String, String)> {
    use std::{process::Command, sync::OnceLock};
    static IDENTITY: OnceLock<std::result::Result<(String, String), String>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| {
            let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
            let output = crate::export_control::hide_child_window(&mut Command::new(rustc))
                .arg("--version")
                .arg("--verbose")
                .output()
                .map_err(|error| format!("reading rustc identity for Sono-GCC cache: {error}"))?;
            if !output.status.success() {
                return Err(
                    "rustc could not report its version for Sono-GCC cache identity".into(),
                );
            }
            let identity = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            let target = identity
                .lines()
                .find_map(|line| line.strip_prefix("host: "))
                .unwrap_or("unknown-target")
                .to_owned();
            Ok((identity, target))
        })
        .clone()
        .map_err(anyhow::Error::msg)
}

#[cfg(windows)]
fn process_cache() -> &'static std::sync::Mutex<HashMap<Vec<u8>, std::sync::Arc<SonoGccProgram>>> {
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<Vec<u8>, std::sync::Arc<SonoGccProgram>>>> =
        OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(windows)]
type CompilationGateRegistry =
    std::sync::Mutex<HashMap<Vec<u8>, std::sync::Weak<std::sync::Mutex<()>>>>;

#[cfg(windows)]
fn compilation_gate(key: &[u8]) -> Result<std::sync::Arc<std::sync::Mutex<()>>> {
    use std::sync::{Mutex, OnceLock, Weak};
    static GATES: OnceLock<CompilationGateRegistry> = OnceLock::new();

    let mut gates = GATES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow::anyhow!("Sono-GCC compilation gate map poisoned"))?;
    gates.retain(|_, gate| gate.strong_count() != 0);
    if let Some(gate) = gates.get(key).and_then(Weak::upgrade) {
        return Ok(gate);
    }
    let gate = std::sync::Arc::new(Mutex::new(()));
    gates.insert(key.to_vec(), std::sync::Arc::downgrade(&gate));
    Ok(gate)
}

#[cfg(windows)]
fn process_cached_program(key: &[u8]) -> Result<Option<std::sync::Arc<SonoGccProgram>>> {
    process_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("Sono-GCC process cache lock poisoned"))
        .map(|entries| entries.get(key).cloned())
}

#[cfg(windows)]
fn insert_process_cache(key: Vec<u8>, program: std::sync::Arc<SonoGccProgram>) -> Result<()> {
    let mut entries = process_cache()
        .lock()
        .map_err(|_| anyhow::anyhow!("Sono-GCC process cache lock poisoned"))?;
    if entries.len() >= 4 && !entries.contains_key(&key) {
        if let Some(evicted) = entries.keys().next().cloned() {
            entries.remove(&evicted);
        }
    }
    entries.insert(key, program);
    Ok(())
}

pub(crate) fn process_cache_info() -> (usize, Option<u64>) {
    #[cfg(windows)]
    {
        let Ok(entries) = process_cache().lock() else {
            return (0, None);
        };
        let bytes = entries.values().fold(0_u64, |total, program| {
            total.saturating_add(
                std::fs::metadata(&program._library.library_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
            )
        });
        (entries.len(), Some(bytes))
    }
    #[cfg(not(windows))]
    {
        (0, Some(0))
    }
}

pub(crate) fn clear_idle_process_cache() -> usize {
    #[cfg(windows)]
    {
        let Ok(mut entries) = process_cache().lock() else {
            return 0;
        };
        let before = entries.len();
        entries.retain(|_, program| std::sync::Arc::strong_count(program) > 1);
        before - entries.len()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

pub(crate) fn evict_process_cache_entry(key: &str) -> Result<bool> {
    #[cfg(windows)]
    {
        let mut cache = process_cache()
            .lock()
            .map_err(|_| anyhow::anyhow!("Sono-GCC process cache lock poisoned"))?;
        let Some(program) = cache.get(key.as_bytes()) else {
            return Ok(false);
        };
        if program._cache_lease.is_some() && std::sync::Arc::strong_count(program) > 1 {
            return Ok(true);
        }
        cache.remove(key.as_bytes());
        Ok(false)
    }
    #[cfg(not(windows))]
    {
        let _ = key;
        Ok(false)
    }
}

#[cfg(windows)]
fn try_disk_cache_lock(
    directory: &std::path::Path,
    key: &str,
) -> Result<crate::cache_manager::EntryLock> {
    let root = directory
        .parent()
        .context("Sono-GCC cache entry has no category directory")?;
    crate::cache_manager::acquire_entry_lock(
        root,
        key,
        crate::cache_manager::EntryLockMode::Exclusive,
    )
    .context("locking Sono-GCC persistent cache entry")
}

#[cfg(windows)]
fn cache_manifest_matches(manifest: &CacheManifest, identity: &CacheIdentity) -> bool {
    manifest.format == CACHE_ENTRY_FORMAT
        && manifest.schema == CACHE_MANIFEST_SCHEMA
        && manifest.cache_key == identity.digest
        && manifest.renderer_version == env!("CARGO_PKG_VERSION")
        && manifest.compiler_backend == COMPILER_BACKEND_VERSION
        && manifest.runtime_abi == COMPILED_RUNTIME_ABI_VERSION
        && manifest.rustc_identity == identity.rustc_identity
        && manifest.target == identity.target
        && manifest.flags == COMPILER_FLAGS_ID
        && manifest.graph_sha1 == identity.graph_sha1
        && manifest.graph_len == identity.graph_bytes.len()
        && manifest.library_len > 0
        && is_single_file_name(&manifest.library_name)
        && manifest.library_name.ends_with(".dll")
        && is_single_file_name(&manifest.metadata_name)
        && manifest.metadata_name.ends_with(".json")
}

#[cfg(windows)]
struct DiskCacheLoad {
    program: SonoGccProgram,
    native_load: Duration,
    disk_validation: Duration,
    metadata_reconstruction: Duration,
}

#[cfg(windows)]
fn load_disk_cache(
    nodes: &[EngineNode],
    identity: &CacheIdentity,
) -> Result<Option<DiskCacheLoad>> {
    let disk_validation_started = Instant::now();
    let root = identity
        .directory
        .parent()
        .context("Sono-GCC cache entry has no category directory")?;
    let Some(cache_lease) = crate::cache_manager::entry_lock_if_present(
        root,
        &identity.digest,
        crate::cache_manager::EntryLockMode::Shared,
    )?
    else {
        return Ok(None);
    };
    let dir_info = match std::fs::symlink_metadata(&identity.directory) {
        Ok(info) if info.file_type().is_dir() && !info.file_type().is_symlink() => info,
        _ => return Ok(None),
    };
    let _ = dir_info;
    let manifest_path = identity.directory.join("manifest.json");
    let graph_path = identity.directory.join("graph.json");
    let manifest_info = match std::fs::symlink_metadata(&manifest_path) {
        Ok(info) if info.file_type().is_file() && !info.file_type().is_symlink() => info,
        _ => return Ok(None),
    };
    if manifest_info.len() > MAX_CACHE_MANIFEST_BYTES {
        return Ok(None);
    }
    let Ok(manifest_bytes) = std::fs::read(&manifest_path) else {
        return Ok(None);
    };
    if manifest_bytes.len() as u64 > MAX_CACHE_MANIFEST_BYTES {
        return Ok(None);
    }
    let Ok(manifest) = serde_json::from_slice::<CacheManifest>(&manifest_bytes) else {
        return Ok(None);
    };
    if !cache_manifest_matches(&manifest, identity) {
        return Ok(None);
    }
    let graph_meta = match std::fs::symlink_metadata(&graph_path) {
        Ok(meta) if meta.file_type().is_file() && !meta.file_type().is_symlink() => meta,
        _ => return Ok(None),
    };
    if graph_meta.len() > MAX_CACHE_PAYLOAD_BYTES
        || graph_meta.len() != identity.graph_bytes.len() as u64
        || std::fs::read(&graph_path).ok().as_deref() != Some(identity.graph_bytes.as_slice())
    {
        return Ok(None);
    }
    let library_path = identity.directory.join(&manifest.library_name);
    let library_meta = match std::fs::symlink_metadata(&library_path) {
        Ok(meta) if meta.file_type().is_file() && !meta.file_type().is_symlink() => meta,
        _ => return Ok(None),
    };
    if library_meta.len() > MAX_CACHE_PAYLOAD_BYTES || library_meta.len() != manifest.library_len {
        return Ok(None);
    }
    let Ok(library_bytes) = std::fs::read(&library_path) else {
        return Ok(None);
    };
    if sha1_hex(&library_bytes) != manifest.library_sha1 {
        return Ok(None);
    }
    let metadata_path = identity.directory.join(&manifest.metadata_name);
    let metadata_meta = match std::fs::symlink_metadata(&metadata_path) {
        Ok(meta) if meta.file_type().is_file() && !meta.file_type().is_symlink() => meta,
        _ => return Ok(None),
    };
    if metadata_meta.len() > MAX_CACHE_PAYLOAD_BYTES
        || metadata_meta.len() != manifest.metadata_len as u64
    {
        return Ok(None);
    }
    let Ok(metadata_bytes) = std::fs::read(&metadata_path) else {
        return Ok(None);
    };
    if metadata_bytes.len() != manifest.metadata_len
        || sha1_hex(&metadata_bytes) != manifest.metadata_sha1
    {
        return Ok(None);
    }
    let disk_validation = disk_validation_started.elapsed();
    let metadata_started = Instant::now();
    let Ok(metadata) = serde_json::from_slice::<CachedProgramMetadata>(&metadata_bytes) else {
        return Ok(None);
    };
    if validate_cached_metadata(&metadata, nodes.len(), &manifest).is_err() {
        return Ok(None);
    }
    let cached_operation_counts = metadata
        .compiled_operations
        .zip(metadata.eligible_operations);
    let units = metadata
        .regions
        .into_iter()
        .map(|region| CompileUnit {
            root: region.root,
            expression: String::new(),
            cut_nodes: region.cut_nodes,
            cut_depths: region.cut_depths,
            metrics: region.metrics,
            function_counts: region.function_counts,
            function_dispatch_count: region.function_dispatch_count,
            compiled_nodes: region.compiled_nodes,
        })
        .collect::<Vec<_>>();
    let metadata_reconstruction = metadata_started.elapsed();
    if units.is_empty() || units.len() != manifest.region_count as usize {
        return Ok(None);
    }
    match load_windows_library(
        nodes.len(),
        units,
        &library_path,
        &identity.directory,
        false,
    ) {
        Ok((mut program, native_load)) => {
            program._cache_lease = Some(cache_lease);
            touch_cache_last_used(&identity.directory);
            if let Some(counts) = cached_operation_counts {
                let _ = program.operation_counts.set(counts);
            }
            Ok(Some(DiskCacheLoad {
                program,
                native_load,
                disk_validation,
                metadata_reconstruction,
            }))
        }
        Err(_) => Ok(None),
    }
}

#[cfg(windows)]
fn persist_disk_cache(
    program: &SonoGccProgram,
    identity: &CacheIdentity,
    compile_time: Duration,
) -> Result<()> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_ID: AtomicU64 = AtomicU64::new(1);

    if identity.graph_bytes.len() as u64 > MAX_CACHE_PAYLOAD_BYTES {
        bail!("serialized Watch graph exceeds the persistent cache payload limit");
    }
    let temp_id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let source_library = &program._library.library_path;
    let library_info =
        std::fs::symlink_metadata(source_library).context("inspecting generated Sono-GCC DLL")?;
    if !library_info.file_type().is_file()
        || library_info.file_type().is_symlink()
        || library_info.len() > MAX_CACHE_PAYLOAD_BYTES
    {
        bail!("generated Sono-GCC DLL exceeds the persistent cache payload limit or has an unsafe type");
    }
    let library_bytes = std::fs::read(source_library).context("reading generated Sono-GCC DLL")?;
    if library_bytes.len() as u64 != library_info.len() {
        bail!("generated Sono-GCC DLL changed while preparing its cache entry");
    }
    let library_sha1 = sha1_hex(&library_bytes);
    let library_name = format!("program-{library_sha1}.dll");
    std::fs::create_dir_all(&identity.directory).context("creating persistent GCC cache entry")?;
    match std::fs::symlink_metadata(&identity.directory) {
        Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {}
        Ok(_) => bail!(
            "refusing to write through non-directory cache entry {}",
            identity.directory.display()
        ),
        Err(error) => return Err(error).context("validating Sono-GCC cache entry directory"),
    }
    // Remove the commit marker first. Every payload is written to a same-volume
    // temporary file and renamed into place; manifest.json is published last.
    // A crash at any earlier point leaves a miss, never a valid half-entry.
    let manifest_path = identity.directory.join("manifest.json");
    match std::fs::remove_file(&manifest_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("invalidating prior GCC cache manifest"),
    }
    cleanup_known_cache_payloads(&identity.directory)?;

    let region_entries = program.regions.iter().collect::<Vec<_>>();
    let cached_metadata = CachedProgramMetadata {
        schema: 1,
        graph_sha1: identity.graph_sha1.clone(),
        compiled_operations: Some(
            program
                .operation_counts
                .get()
                .context("Sono-GCC operation summary was not computed before cache write")?
                .0,
        ),
        eligible_operations: Some(
            program
                .operation_counts
                .get()
                .context("Sono-GCC operation summary was not computed before cache write")?
                .1,
        ),
        regions: region_entries
            .into_iter()
            .map(|region| CachedRegionMetadata {
                root: region.root,
                cut_nodes: region.cut_nodes.clone(),
                cut_depths: region.cut_depths.clone(),
                metrics: region.metrics,
                function_counts: region.function_counts.clone(),
                function_dispatch_count: region.function_dispatch_count,
                compiled_nodes: region.compiled_nodes.clone(),
            })
            .collect(),
    };
    let metadata_bytes = serde_json::to_vec(&cached_metadata)
        .context("serializing Sono-GCC cached region metadata")?;
    if metadata_bytes.len() as u64 > MAX_CACHE_PAYLOAD_BYTES {
        bail!("Sono-GCC region metadata exceeds the persistent cache payload limit");
    }
    let metadata_sha1 = sha1_hex(&metadata_bytes);
    let metadata_name = "regions.json".to_owned();
    atomic_cache_file(&identity.directory, &library_name, &library_bytes, temp_id)?;
    atomic_cache_file(
        &identity.directory,
        "graph.json",
        &identity.graph_bytes,
        temp_id,
    )?;
    atomic_cache_file(
        &identity.directory,
        &metadata_name,
        &metadata_bytes,
        temp_id,
    )?;

    let manifest = CacheManifest {
        format: CACHE_ENTRY_FORMAT.to_owned(),
        schema: CACHE_MANIFEST_SCHEMA,
        cache_key: identity.digest.clone(),
        renderer_version: env!("CARGO_PKG_VERSION").to_owned(),
        created_unix_ms: crate::cache_manager::now_unix_ms(),
        compiler_backend: COMPILER_BACKEND_VERSION.to_owned(),
        runtime_abi: COMPILED_RUNTIME_ABI_VERSION.to_owned(),
        rustc_identity: identity.rustc_identity.clone(),
        target: identity.target.clone(),
        flags: COMPILER_FLAGS_ID.to_owned(),
        graph_sha1: identity.graph_sha1.clone(),
        graph_len: identity.graph_bytes.len(),
        library_name,
        library_sha1,
        library_len: library_bytes.len() as u64,
        metadata_name,
        metadata_sha1,
        metadata_len: metadata_bytes.len(),
        region_count: program.regions.len() as u64,
        original_compile_ms: compile_time.as_millis(),
    };
    let manifest_bytes = serde_json::to_vec(&manifest).context("serializing GCC cache manifest")?;
    atomic_cache_file(
        &identity.directory,
        "manifest.json",
        &manifest_bytes,
        temp_id,
    )?;
    Ok(())
}

#[cfg(windows)]
fn atomic_cache_file(
    directory: &std::path::Path,
    name: &str,
    bytes: &[u8],
    temp_id: u64,
) -> Result<()> {
    use std::io::Write;
    let temp = directory.join(format!(".{name}.{}-{temp_id}.tmp", std::process::id()));
    let final_path = directory.join(name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .with_context(|| format!("creating temporary cache artifact {}", temp.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("writing temporary cache artifact {}", temp.display()))?;
    file.sync_all()
        .with_context(|| format!("flushing temporary cache artifact {}", temp.display()))?;
    drop(file);
    std::fs::rename(&temp, &final_path).with_context(|| {
        format!(
            "atomically publishing cache artifact {}",
            final_path.display()
        )
    })
}

#[cfg(windows)]
fn cleanup_known_cache_payloads(directory: &std::path::Path) -> Result<()> {
    for item in std::fs::read_dir(directory)
        .with_context(|| format!("cleaning cache entry {}", directory.display()))?
    {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        let known = name == "graph.json"
            || name == "regions.json"
            || name == "last-used"
            || name == "compile.lock"
            || (name.starts_with("program-") && name.ends_with(".dll"))
            || (name.starts_with('.') && name.ends_with(".tmp"));
        if !known {
            continue;
        }
        let kind = item.file_type()?;
        if kind.is_symlink() {
            bail!(
                "refusing to rewrite cache entry containing a symlink at {}",
                item.path().display()
            );
        } else if kind.is_file() {
            std::fs::remove_file(item.path())
                .with_context(|| format!("removing old cache payload {}", item.path().display()))?;
        } else {
            bail!(
                "refusing to rewrite cache entry with unexpected payload {}",
                item.path().display()
            );
        }
    }
    Ok(())
}

#[cfg(windows)]
fn compile_cached_windows<F>(
    nodes: &[EngineNode],
    on_compile_start: F,
) -> Result<SonoGccCompileResult>
where
    F: FnOnce(),
{
    let lookup_started = Instant::now();
    let mut identity = cache_identity(nodes)?;
    let cache_key = identity.digest.as_bytes().to_vec();

    if let Some(program) = process_cached_program(&cache_key)? {
        if program._cache_lease.is_some() {
            touch_cache_last_used(&program._library.directory);
        }
        let timings = CompileTimings {
            cache_lookup: lookup_started.elapsed(),
            graph_identity: identity.identity_time,
            ..CompileTimings::default()
        };
        return Ok(compile_result(
            program,
            identity.digest,
            CacheHit::Process,
            timings,
            nodes,
        ));
    }

    // Serialize same-process cold requests even when neither persistent store
    // is writable. The weak registry retains only currently active identities.
    let gate = compilation_gate(&cache_key)?;
    let _gate_guard = gate
        .lock()
        .map_err(|_| anyhow::anyhow!("Sono-GCC per-graph compilation gate poisoned"))?;
    if let Some(program) = process_cached_program(&cache_key)? {
        if program._cache_lease.is_some() {
            touch_cache_last_used(&program._library.directory);
        }
        return Ok(compile_result(
            program,
            identity.digest,
            CacheHit::Process,
            CompileTimings {
                cache_lookup: lookup_started.elapsed(),
                graph_identity: identity.identity_time,
                ..CompileTimings::default()
            },
            nodes,
        ));
    }

    if let Some(cache) = load_disk_cache(nodes, &identity)? {
        let DiskCacheLoad {
            program,
            native_load,
            disk_validation,
            metadata_reconstruction,
        } = cache;
        let program = std::sync::Arc::new(program);
        insert_process_cache(cache_key, program.clone())?;
        let timings = CompileTimings {
            cache_lookup: lookup_started.elapsed().saturating_sub(native_load),
            native_load,
            graph_identity: identity.identity_time,
            disk_validation,
            metadata_reconstruction,
            ..CompileTimings::default()
        };
        return Ok(compile_result(
            program,
            identity.digest,
            CacheHit::Persistent,
            timings,
            nodes,
        ));
    }

    let mut disk_lock = match try_disk_cache_lock(&identity.directory, &identity.digest) {
        Ok(lock) => Some(lock),
        Err(primary_error) => {
            identity.directory = fallback_cache_root().join(&identity.digest);
            match try_disk_cache_lock(&identity.directory, &identity.digest) {
                Ok(lock) => Some(lock),
                Err(fallback_error) => {
                    eprintln!(
                        "Sono-GCC persistent cache unavailable ({}; {}); compilation remains process-local",
                        primary_error, fallback_error
                    );
                    None
                }
            }
        }
    };
    // Requests for the same key wait on the per-entry OS lock. After waiting,
    // recheck the process cache and the persistent payload before compiling.
    for _ in 0..3 {
        if let Some(program) = process_cached_program(&cache_key)? {
            if program._cache_lease.is_some() {
                touch_cache_last_used(&program._library.directory);
            }
            return Ok(compile_result(
                program,
                identity.digest,
                CacheHit::Process,
                CompileTimings {
                    cache_lookup: lookup_started.elapsed(),
                    graph_identity: identity.identity_time,
                    ..CompileTimings::default()
                },
                nodes,
            ));
        }
        let persisted_valid = inspect_cache_payload(&identity.directory, &identity.digest)
            .is_ok_and(|(_, compatible)| compatible);
        if !persisted_valid || disk_lock.is_none() {
            break;
        }
        disk_lock.take().unwrap().unlock()?;
        if let Some(cache) = load_disk_cache(nodes, &identity)? {
            let DiskCacheLoad {
                program,
                native_load,
                disk_validation,
                metadata_reconstruction,
            } = cache;
            let program = std::sync::Arc::new(program);
            insert_process_cache(cache_key.clone(), program.clone())?;
            let timings = CompileTimings {
                cache_lookup: lookup_started.elapsed().saturating_sub(native_load),
                native_load,
                graph_identity: identity.identity_time,
                disk_validation,
                metadata_reconstruction,
                ..CompileTimings::default()
            };
            return Ok(compile_result(
                program,
                identity.digest,
                CacheHit::Persistent,
                timings,
                nodes,
            ));
        }
        disk_lock = Some(try_disk_cache_lock(&identity.directory, &identity.digest)?);
    }

    on_compile_start();
    let compile_started = Instant::now();
    let (compiled, total, region_codegen, rustc_compile) = SonoGccProgram::compile(nodes)?;
    let native_load = compiled.native_load_time();
    let compilation = total.saturating_sub(native_load);
    let operation_summary_started = Instant::now();
    let _ = compiled.operation_counts(nodes);
    let operation_summary = operation_summary_started.elapsed();
    let cache_write_started = Instant::now();
    let cache_write_result = if disk_lock.is_some() {
        persist_disk_cache(&compiled, &identity, compilation)
    } else {
        Err(anyhow::anyhow!(
            "persistent cache is not writable; this program remains cached only in process"
        ))
    };
    let cache_write = cache_write_started.elapsed();
    let program = std::sync::Arc::new(compiled);
    insert_process_cache(cache_key, program.clone())?;
    let timings = CompileTimings {
        cache_lookup: lookup_started
            .elapsed()
            .saturating_sub(compile_started.elapsed()),
        compilation,
        region_codegen,
        rustc_compile,
        native_load,
        cache_write,
        graph_identity: identity.identity_time,
        ..CompileTimings::default()
    };
    let mut result = compile_result(program, identity.digest, CacheHit::None, timings, nodes);
    result.timings.operation_summary += operation_summary;
    result.cache_write_error = cache_write_result.err().map(|error| format!("{error:#}"));
    Ok(result)
}

#[cfg(windows)]
fn compile_result(
    program: std::sync::Arc<SonoGccProgram>,
    cache_key: String,
    cache_hit: CacheHit,
    mut timings: CompileTimings,
    nodes: &[EngineNode],
) -> SonoGccCompileResult {
    let summary_started = Instant::now();
    let (compiled_operations, eligible_operations) = program.operation_counts(nodes);
    timings.operation_summary = summary_started.elapsed();
    SonoGccCompileResult {
        program,
        cache_key,
        cache_hit,
        timings,
        graph_nodes: nodes.len(),
        compiled_operations,
        eligible_operations,
        cache_write_error: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};
    use std::sync::Arc;

    #[test]
    fn region_size_buckets_cover_boundaries_without_overlap() {
        let cases = [
            (1, 0),
            (2, 1),
            (3, 2),
            (4, 2),
            (5, 3),
            (8, 3),
            (9, 4),
            (16, 4),
            (17, 5),
            (32, 5),
            (33, 6),
            (64, 6),
            (65, 7),
        ];
        for (size, expected_bucket) in cases {
            assert_eq!(region_size_bucket(size), expected_bucket, "size {size}");
        }
    }

    #[test]
    fn dispatch_policy_leaves_single_operation_regions_to_watch_vm() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value": 1.0}, {"value": 2.0}, {"func": "Add", "args": [0, 1]},
            {"value": 3.0}, {"value": 4.0}, {"func": "Add", "args": [3, 4]},
            {"value": 5.0}, {"func": "Multiply", "args": [5, 6]}
        ]))
        .unwrap();
        let units = compile_units(&nodes).unwrap();
        assert_eq!(
            units
                .iter()
                .map(|unit| unit.compiled_nodes.len())
                .collect::<Vec<_>>(),
            [1, 2]
        );

        let selected = dispatchable_units(units);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].root, 7);
        assert_eq!(selected[0].compiled_nodes.len(), 2);
    }

    fn append_node(nodes: &mut Vec<EngineNode>, node: Value) -> usize {
        let index = nodes.len();
        nodes.push(serde_json::from_value(node).unwrap());
        index
    }

    #[test]
    fn operation_classifier_keeps_runtime_ops_in_vm_cuts() {
        let nodes: Vec<EngineNode> = serde_json::from_value(serde_json::json!([
            {"value":1}, {"value":2}, {"func":"Add", "args":[0,1]},
            {"func":"Draw", "args":[2]}, {"func":"Set", "args":[0,1,2]},
            {"func":"If", "args":[0,1,2]}, {"func":"JumpLoop", "args":[0]}
        ]))
        .unwrap();
        let mut states = vec![0; nodes.len()];
        assert!(eligible(&nodes, 2, &mut states, 0));
        for root in [3, 4, 5, 6] {
            let mut states = vec![0; nodes.len()];
            assert!(eligible(&nodes, root, &mut states, 0));
            assert!(is_vm_cut(&nodes, root, &nodes[root]));
        }
        let units = compile_units(&nodes).unwrap();
        assert!(units.iter().any(|unit| unit.root == 2));
        assert!([3, 4, 5, 6]
            .into_iter()
            .all(|root| units.iter().all(|unit| unit.root != root)));
    }

    #[test]
    fn vm_cut_inventory_covers_runtime_operations_and_all_easings() {
        let names = VM_CUT_FUNCTIONS.iter().copied().collect::<HashSet<_>>();
        assert_eq!(VM_CUT_FUNCTIONS.len(), 88);
        assert_eq!(names.len(), VM_CUT_FUNCTIONS.len());
        let easing_names = ["In", "Out", "InOut", "OutIn"]
            .into_iter()
            .flat_map(|direction| {
                [
                    "Sine", "Quad", "Cubic", "Quart", "Quint", "Expo", "Circ", "Back", "Elastic",
                ]
                .into_iter()
                .map(move |family| format!("Ease{direction}{family}"))
            })
            .collect::<HashSet<_>>();
        assert_eq!(easing_names.len(), 36);
        assert!(easing_names.iter().all(|name| is_vm_cut_function(name)));
        let additional_easings = easing_names
            .iter()
            .filter(|name| !names.contains(name.as_str()))
            .count();
        assert_eq!(additional_easings, 32);
        assert_eq!(
            NATIVE_FUNCTION_COUNT + names.len() + additional_easings,
            161
        );
        assert!(NATIVE_FUNCTION_NAMES
            .iter()
            .all(|name| !names.contains(name) && !is_vm_cut_function(name)));
    }

    #[test]
    fn watch_memory_operations_stay_on_the_shared_vm_path_for_sono_gcc() {
        for name in [
            "Get",
            "GetPointed",
            "GetShifted",
            "Set",
            "SetPointed",
            "SetShifted",
            "SetAdd",
            "SetAddPointed",
            "SetAddShifted",
        ] {
            assert!(
                is_vm_cut_function(name),
                "{name} must use WatchVm memory handling"
            );
        }
    }

    #[test]
    fn native_function_slots_are_unique_and_cover_direct_operations() {
        let names = NATIVE_FUNCTION_NAMES
            .iter()
            .copied()
            .collect::<HashSet<_>>();
        assert_eq!(NATIVE_FUNCTION_NAMES.len(), NATIVE_FUNCTION_COUNT);
        assert_eq!(names.len(), NATIVE_FUNCTION_COUNT);
        for (index, name) in NATIVE_FUNCTION_NAMES.iter().enumerate() {
            assert_eq!(native_function_name(index), *name);
            assert!(!is_vm_cut_function(name));
        }
    }

    #[cfg(windows)]
    #[test]
    fn process_cache_probe_misses_without_compiling() {
        let mut nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":1}, {"value":2}, {"func":"Add", "args":[0,1]}
        ]))
        .unwrap();
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos() as f64;
        append_node(&mut nodes, json!({"value":nonce}));
        assert!(SonoGccProgram::compile_if_process_cached(&nodes)
            .unwrap()
            .is_none());
    }

    #[cfg(windows)]
    #[test]
    fn compiler_tests_use_an_isolated_temporary_cache_root() {
        let test_root = test_cache_base();
        assert!(cache_root().starts_with(&test_root));
        assert!(fallback_cache_root().starts_with(test_root));
    }

    #[test]
    fn easing_classifier_keeps_every_easing_path_on_watch_vm() {
        for direction in ["In", "Out", "InOut", "OutIn"] {
            for family in [
                "Sine", "Quad", "Cubic", "Quart", "Quint", "Expo", "Circ", "Back", "Elastic",
            ] {
                let name = format!("Ease{direction}{family}");
                assert!(ease_parts(&name).is_some());
                let nodes: Vec<EngineNode> = serde_json::from_value(json!([
                    {"value":0.5}, {"func":name, "args":[0]}
                ]))
                .unwrap();
                assert!(
                    is_vm_cut(&nodes, 1, &nodes[1]),
                    "{name} must remain in WatchVm"
                );
            }
        }
        assert!(ease_parts("EaseInElasticExtra").is_none());
    }

    #[cfg(windows)]
    #[test]
    fn elastic_nan_uses_watch_vm_even_when_a_native_program_is_loaded() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":7777}, {"value":0}, {"func":"Get", "args":[0,1]},
            {"func":"EaseInElastic", "args":[2]},
            {"value":1}, {"value":2}, {"func":"Add", "args":[4,5]},
            {"value":3}, {"func":"Add", "args":[6,7]}
        ]))
        .unwrap();
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        assert!(
            program.region(8).is_some(),
            "the test must load native code"
        );
        assert!(program.region(3).is_none(), "Elastic must remain a VM cut");

        let payload_nan = f64::from_bits(0x7ff8_0000_0000_0042);
        let mut vm = crate::runtime::WatchVm::new(&nodes);
        vm.memory.set(7777, 0, payload_nan);
        let expected = vm.execute(3).unwrap();
        assert!(expected.is_nan());

        let mut gcc = crate::runtime::WatchVm::new(&nodes);
        gcc.memory = vm.memory.clone();
        gcc.set_sono_gcc_program(Some(Arc::new(program)));
        let actual = gcc.execute(3).unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits());
        assert_eq!(gcc.sono_gcc_regions_executed(), 0);
        assert_eq!(gcc.sono_gcc_vm_to_native_cut_calls(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn exact_graph_cache_identity_changes_when_watch_values_change() {
        let first: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":1}, {"value":2}, {"func":"Add", "args":[0,1]}
        ]))
        .unwrap();
        let second: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":1}, {"value":3}, {"func":"Add", "args":[0,1]}
        ]))
        .unwrap();
        let identity = cache_identity(&first).unwrap();
        let header = format!(
            "{COMPILER_BACKEND_VERSION}\0{COMPILED_RUNTIME_ABI_VERSION}\0{}\0{COMPILER_FLAGS_ID}\0{}\0{}\0",
            env!("CARGO_PKG_VERSION"),
            identity.target,
            identity.rustc_identity
        );
        let mut legacy_identity_bytes = header.into_bytes();
        legacy_identity_bytes.extend_from_slice(&identity.graph_bytes);
        assert_eq!(identity.digest, sha1_hex(&legacy_identity_bytes));
        assert_ne!(identity.digest, cache_identity(&second).unwrap().digest);
    }

    #[cfg(windows)]
    #[test]
    fn persistent_manifest_rejects_backend_runtime_abi_compiler_and_flag_changes() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":1}, {"value":2}, {"func":"Add", "args":[0,1]}
        ]))
        .unwrap();
        let identity = cache_identity(&nodes).unwrap();
        let manifest = CacheManifest {
            format: CACHE_ENTRY_FORMAT.to_owned(),
            schema: CACHE_MANIFEST_SCHEMA,
            cache_key: identity.digest.clone(),
            renderer_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_unix_ms: crate::cache_manager::now_unix_ms(),
            compiler_backend: COMPILER_BACKEND_VERSION.to_owned(),
            runtime_abi: COMPILED_RUNTIME_ABI_VERSION.to_owned(),
            rustc_identity: identity.rustc_identity.clone(),
            target: identity.target.clone(),
            flags: COMPILER_FLAGS_ID.to_owned(),
            graph_sha1: identity.graph_sha1.clone(),
            graph_len: identity.graph_bytes.len(),
            library_name: "program-test.dll".to_owned(),
            library_sha1: "checksum".to_owned(),
            library_len: 1,
            metadata_name: "regions.json".to_owned(),
            metadata_sha1: "metadata-checksum".to_owned(),
            metadata_len: 42,
            region_count: 1,
            original_compile_ms: 0,
        };
        assert!(cache_manifest_matches(&manifest, &identity));
        let mut stale = manifest;
        stale.compiler_backend.push_str("-old");
        assert!(!cache_manifest_matches(&stale, &identity));
        stale.compiler_backend = COMPILER_BACKEND_VERSION.to_owned();
        stale.runtime_abi.push_str("-old");
        assert!(!cache_manifest_matches(&stale, &identity));
        stale.runtime_abi = COMPILED_RUNTIME_ABI_VERSION.to_owned();
        stale.flags.push_str("-old");
        assert!(!cache_manifest_matches(&stale, &identity));
        stale.flags = COMPILER_FLAGS_ID.to_owned();
        stale.rustc_identity.push_str("-old");
        assert!(!cache_manifest_matches(&stale, &identity));
        stale.rustc_identity = identity.rustc_identity.clone();
        stale.library_name = "..\\outside.dll".to_owned();
        assert!(!cache_manifest_matches(&stale, &identity));
        stale.library_name = "program-test.dll".to_owned();
        stale.metadata_name = "..\\outside.json".to_owned();
        assert!(!cache_manifest_matches(&stale, &identity));
        assert!(!is_single_file_name("../outside.dll"));
        assert!(!is_single_file_name("C:\\outside.dll"));
        assert!(is_single_file_name("program-cache.dll"));
    }

    #[cfg(windows)]
    #[test]
    fn cache_payload_write_uses_atomic_same_directory_publication() {
        let directory = tempfile::tempdir().unwrap();
        atomic_cache_file(directory.path(), "graph.json", b"graph-payload", 99).unwrap();
        assert_eq!(
            std::fs::read(directory.path().join("graph.json")).unwrap(),
            b"graph-payload"
        );
        let leftovers = std::fs::read_dir(directory.path())
            .unwrap()
            .map(|item| item.unwrap().file_name())
            .collect::<Vec<_>>();
        assert_eq!(leftovers, [std::ffi::OsString::from("graph.json")]);
        cleanup_known_cache_payloads(directory.path()).unwrap();
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn unknown_manifest_schema_is_rejected_before_payload_deserialization() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("manifest.json"),
            br#"{"schema":3,"target":"old-target"}"#,
        )
        .unwrap();
        let error =
            inspect_cache_manifest(directory.path(), "0123456789abcdef0123456789abcdef01234567")
                .unwrap_err();
        assert!(error.to_string().contains("older or unknown schema"));
    }

    #[cfg(windows)]
    #[test]
    fn oversized_cache_manifest_is_rejected_before_payload_access() {
        let directory = tempfile::tempdir().unwrap();
        let key = "0123456789abcdef0123456789abcdef01234567";
        let value = serde_json::json!({
            "format": CACHE_ENTRY_FORMAT,
            "schema": CACHE_MANIFEST_SCHEMA,
            "cache_key": key,
            "renderer_version": env!("CARGO_PKG_VERSION"),
            "created_unix_ms": 0,
            "compiler_backend": COMPILER_BACKEND_VERSION,
            "runtime_abi": COMPILED_RUNTIME_ABI_VERSION,
            "rustc_identity": "test rustc",
            "target": "x86_64-pc-windows-msvc",
            "flags": COMPILER_FLAGS_ID,
            "graph_sha1": "0".repeat(40),
            "graph_len": 1,
            "library_name": "program.dll",
            "library_sha1": "0".repeat(40),
            "library_len": MAX_CACHE_PAYLOAD_BYTES + 1,
            "metadata_name": "regions.json",
            "metadata_sha1": "0".repeat(40),
            "metadata_len": 1,
            "region_count": 0,
            "original_compile_ms": 0
        });
        std::fs::write(
            directory.path().join("manifest.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();

        let error = inspect_cache_manifest(directory.path(), key).unwrap_err();
        assert!(error.to_string().contains("payload size is invalid"));
    }

    #[cfg(windows)]
    #[test]
    fn structurally_invalid_region_metadata_is_never_accepted() {
        let manifest = CacheManifest {
            format: CACHE_ENTRY_FORMAT.to_owned(),
            schema: CACHE_MANIFEST_SCHEMA,
            cache_key: "0".repeat(40),
            renderer_version: env!("CARGO_PKG_VERSION").to_owned(),
            created_unix_ms: 0,
            compiler_backend: COMPILER_BACKEND_VERSION.to_owned(),
            runtime_abi: COMPILED_RUNTIME_ABI_VERSION.to_owned(),
            rustc_identity: "test".into(),
            target: "test".into(),
            flags: COMPILER_FLAGS_ID.to_owned(),
            graph_sha1: "graph".into(),
            graph_len: 1,
            library_name: "program.dll".into(),
            library_sha1: "library".into(),
            library_len: 1,
            metadata_name: "regions.json".into(),
            metadata_sha1: "metadata".into(),
            metadata_len: 1,
            region_count: 0,
            original_compile_ms: 0,
        };
        let metadata = CachedProgramMetadata {
            schema: 1,
            graph_sha1: "graph".into(),
            compiled_operations: None,
            eligible_operations: None,
            regions: Vec::new(),
        };
        assert!(validate_cached_metadata(&metadata, 1, &manifest).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn persistent_region_metadata_accepts_legacy_entries_without_summary_counts() {
        let metadata = CachedProgramMetadata {
            schema: 1,
            graph_sha1: "graph".to_owned(),
            compiled_operations: None,
            eligible_operations: None,
            regions: Vec::new(),
        };
        let mut value = serde_json::to_value(metadata).unwrap();
        value.as_object_mut().unwrap().remove("compiled_operations");
        value.as_object_mut().unwrap().remove("eligible_operations");
        let legacy: CachedProgramMetadata = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.compiled_operations, None);
        assert_eq!(legacy.eligible_operations, None);
    }

    #[cfg(windows)]
    #[test]
    fn cold_compilation_gates_share_same_key_and_allow_unrelated_keys() {
        let first = compilation_gate(b"graph-a").unwrap();
        let same = compilation_gate(b"graph-a").unwrap();
        let other = compilation_gate(b"graph-b").unwrap();
        assert!(Arc::ptr_eq(&first, &same));
        assert!(!Arc::ptr_eq(&first, &other));

        let held = first.lock().unwrap();
        assert!(same.try_lock().is_err());
        assert!(other.try_lock().is_ok());
        drop(held);
        assert!(same.try_lock().is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_precompile_requests_share_one_safe_exact_graph_compile() {
        use std::{
            sync::atomic::{AtomicBool, Ordering},
            sync::mpsc,
            time::{SystemTime, UNIX_EPOCH},
        };

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as u64
            ^ u64::from(std::process::id());
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":nonce}, {"value":1}, {"func":"Add", "args":[0,1]},
            {"func":"Add", "args":[2,1]}
        ]))
        .unwrap();
        let worker_nodes = nodes.clone();
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            precompile_with_progress(&worker_nodes, move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
        });
        started_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the cache miss should reach the compile stage");
        let duplicate_started = Arc::new(AtomicBool::new(false));
        let duplicate_started_in_thread = Arc::clone(&duplicate_started);
        let duplicate_nodes = nodes.clone();
        let duplicate = std::thread::spawn(move || {
            precompile_with_progress(&duplicate_nodes, move || {
                duplicate_started_in_thread.store(true, Ordering::SeqCst);
            })
        });
        std::thread::sleep(Duration::from_millis(50));
        assert!(!duplicate_started.load(Ordering::SeqCst));
        release_tx.send(()).unwrap();
        let first = worker.join().unwrap().unwrap();
        let second = duplicate.join().unwrap().unwrap();
        assert!(
            !duplicate_started.load(Ordering::SeqCst),
            "the waiting request should reuse the completed compilation"
        );
        assert_eq!(second.cache_hit, CacheHit::Process);
        assert_eq!(first.cache_key, second.cache_key);
        assert_eq!(first.compiled_operations, second.compiled_operations);
        assert_eq!(first.eligible_operations, second.eligible_operations);
        let cached = SonoGccProgram::compile_if_process_cached(&nodes)
            .unwrap()
            .unwrap();
        let cached_again = SonoGccProgram::compile_if_process_cached(&nodes)
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&cached.program, &cached_again.program));
        assert_eq!(
            cached.program.operation_counts.get(),
            cached_again.program.operation_counts.get()
        );
    }

    #[test]
    fn constants_are_encoded_by_exact_float_bits() {
        let value = -0.0_f64;
        let source = format!("f64::from_bits(0x{:016x})", value.to_bits());
        assert!(source.contains("8000000000000000"));
    }

    #[cfg(windows)]
    #[test]
    fn native_regions_and_vm_cuts_match_watch_vm_over_adversarial_numbers() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut operations = [
            "Add",
            "Multiply",
            "Min",
            "Max",
            "Rem",
            "Subtract",
            "Divide",
            "Power",
            "Clamp",
            "Lerp",
            "LerpClamped",
            "Unlerp",
            "UnlerpClamped",
            "Remap",
            "RemapClamped",
            "Equal",
            "NotEqual",
            "Greater",
            "GreaterOr",
            "Less",
            "LessOr",
            "Arctan2",
            "Abs",
            "Arctan",
            "Ceil",
            "Cos",
            "Floor",
            "Log",
            "Negate",
            "Not",
            "Round",
            "Sin",
            "Trunc",
            "Frac",
            "Sign",
            "Radian",
            "Degree",
            "Arcsin",
            "Arccos",
            "Tan",
            "Cosh",
            "Sinh",
            "Tanh",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        for direction in ["In", "Out", "InOut", "OutIn"] {
            for family in [
                "Sine", "Quad", "Cubic", "Quart", "Quint", "Expo", "Circ", "Back",
            ] {
                operations.push(format!("Ease{direction}{family}"));
            }
        }
        let variadic = [
            "Add", "Multiply", "Min", "Max", "Rem", "Subtract", "Divide", "Power",
        ];
        let binary = [
            "Equal",
            "NotEqual",
            "Greater",
            "GreaterOr",
            "Less",
            "LessOr",
            "Arctan2",
        ];
        let ternary = ["Clamp", "Lerp", "LerpClamped", "Unlerp", "UnlerpClamped"];
        let remap = ["Remap", "RemapClamped"];
        let mut nodes = Vec::new();
        let mut inputs = Vec::new();
        for slot in 0..5 {
            let block = append_node(&mut nodes, json!({"value": 7777}));
            let index = append_node(&mut nodes, json!({"value": slot}));
            inputs.push(append_node(
                &mut nodes,
                json!({"func":"Get", "args":[block,index]}),
            ));
        }
        let mut roots = Vec::new();
        for operation in &operations {
            let count = if variadic.contains(&operation.as_str()) {
                4
            } else if remap.contains(&operation.as_str()) {
                5
            } else if ternary.contains(&operation.as_str()) {
                3
            } else if binary.contains(&operation.as_str()) {
                2
            } else {
                1
            };
            let args = inputs.iter().take(count).copied().collect::<Vec<_>>();
            let operation_node = append_node(&mut nodes, json!({"func":operation, "args":args}));
            let mut root = operation_node;
            for _ in 0..4 {
                root = append_node(&mut nodes, json!({"func":"Negate", "args":[root]}));
            }
            roots.push((operation.clone(), operation_node, root));
        }
        // Keep this differential run independent of a previous DLL cached for
        // the same synthetic graph, so it always exercises fresh compilation.
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos() as f64;
        append_node(&mut nodes, json!({"value":nonce}));

        let (program, compile_time, cache_hit) = SonoGccProgram::compile_cached(&nodes).unwrap();
        assert!(compile_time > Duration::ZERO);
        assert!(!cache_hit);
        let (cached, _, cache_hit) = SonoGccProgram::compile_cached(&nodes).unwrap();
        assert!(cache_hit);
        assert!(Arc::ptr_eq(&program, &cached));
        assert_eq!(program.region_count(), roots.len());
        for (operation, operation_node, root) in &roots {
            let region = program
                .region(*root)
                .expect("the final Negates form a native region");
            let direct_native = !is_vm_cut(&nodes, *operation_node, &nodes[*operation_node]);
            assert_eq!(
                region.compiled_nodes.contains(operation_node),
                direct_native,
                "{operation} must use its declared native or WatchVm path"
            );
        }
        let cases = [
            [0.0, -0.0, 1.5, -2.25, 0.0],
            [1.0, 0.5, -1.0, 2.0, -0.0],
            [f64::MIN_POSITIVE, f64::MAX / 2.0, 1.0, 3.0, f64::MIN],
            [f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0, f64::MAX],
            [
                f64::from_bits(0x7ff8_0000_0000_0042),
                f64::from_bits(0x7ff8_0000_0000_0001),
                0.0,
                -0.0,
                f64::NEG_INFINITY,
            ],
            [
                f64::from_bits(0xfff8_0000_0000_1234),
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::from_bits(0x7ff8_0000_0000_00a5),
                -3.0,
            ],
            [9.75, -3.5, 2.0, 0.5, f64::EPSILON],
            [2.0, 4.0, -8.0, 3.0, f64::MAX],
            [
                f64::from_bits(1),
                f64::from_bits(0x8000_0000_0000_0001),
                -1.0,
                1.0,
                0.0,
            ],
        ];
        for case in cases {
            let mut memory = crate::runtime::Memory::new();
            for (slot, value) in case.into_iter().enumerate() {
                memory.set(7777, slot, value);
            }
            for (operation, _, root) in &roots {
                let mut vm = crate::runtime::WatchVm::new(&nodes);
                vm.memory = memory.clone();
                let expected = vm.execute(*root).unwrap();
                let expected_evaluations = vm.evaluation_count();
                let expected_counts = vm.function_counts.clone();

                let mut gcc = crate::runtime::WatchVm::new(&nodes);
                gcc.memory = memory.clone();
                gcc.set_sono_gcc_program(Some(program.clone()));
                let actual = gcc.execute(*root).unwrap();
                assert_eq!(
                    actual.to_bits(),
                    expected.to_bits(),
                    "{operation} mismatch for inputs {case:?}: VM={expected:?}, GCC={actual:?}"
                );
                assert_eq!(
                    gcc.evaluation_count(),
                    expected_evaluations,
                    "{operation} evaluation accounting"
                );
                assert_eq!(
                    gcc.function_counts, expected_counts,
                    "{operation} function accounting"
                );
                for slot in 0..5 {
                    assert_eq!(
                        gcc.memory.get(7777, slot).to_bits(),
                        memory.get(7777, slot).to_bits()
                    );
                }
                assert_eq!(
                    gcc.display_list.sprites.len(),
                    vm.display_list.sprites.len()
                );
            }
        }
    }

    #[cfg(windows)]
    #[test]
    fn runtime_cuts_preserve_order_lazy_effects_memory_and_display_state() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":7777}, {"value":0}, {"value":4},
            {"func":"Set", "args":[0,1,2]},
            {"func":"Get", "args":[0,1]},
            {"value":7}, {"value":12.5}, {"func":"Play", "args":[5,6]},
            {"value":1}, {"value":2}, {"value":99}, {"value":0.5},
            {"func":"Play", "args":[10,11]}, {"func":"If", "args":[8,9,12]},
            {"value":1}, {"value":0}, {"value":0}, {"value":1},
            {"value":1}, {"value":1}, {"value":0}, {"value":1},
            {"value":0}, {"value":0}, {"value":1}, {"value":0},
            {"func":"Draw", "args":[14,15,16,17,18,19,20,21,22,23,24]},
            {"value":3}, {"value":8}, {"func":"Spawn", "args":[27,28]},
            {"func":"Add", "args":[3,4,7,13,26,29]},
            {"value":0}, {"func":"Add", "args":[30,31]}
        ]))
        .unwrap();
        let root = nodes.len() - 1;
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        let region = program.region(root).expect("mixed native root is compiled");
        assert_eq!(region.cut_nodes, [3, 4, 7, 13, 26, 29]);

        let mut vm = crate::runtime::WatchVm::new(&nodes);
        vm.context.time = 42.0;
        let expected = vm.execute(root).unwrap();

        let mut gcc = crate::runtime::WatchVm::new(&nodes);
        gcc.context.time = 42.0;
        gcc.set_sono_gcc_program(Some(Arc::new(program)));
        let actual = gcc.execute(root).unwrap();

        assert_eq!(actual.to_bits(), expected.to_bits());
        assert_eq!(gcc.evaluation_count(), vm.evaluation_count());
        assert_eq!(gcc.function_counts, vm.function_counts);
        assert_eq!(
            gcc.memory.get(7777, 0).to_bits(),
            vm.memory.get(7777, 0).to_bits()
        );
        assert_eq!(gcc.audio_events, vm.audio_events);
        assert_eq!(gcc.spawn_queue, vm.spawn_queue);
        assert_eq!(gcc.display_list, vm.display_list);
        assert_eq!(
            gcc.audio_events.len(),
            1,
            "the inactive If branch must stay lazy"
        );
        assert_eq!(gcc.sono_gcc_regions_executed(), 1);
        assert_eq!(gcc.sono_gcc_vm_to_native_cut_calls(), 6);
        assert_eq!(gcc.sono_gcc_overflow_input_regions(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn native_region_overflow_inputs_match_watch_vm() {
        let mut nodes = Vec::new();
        let mut cuts = Vec::new();
        for slot in 0..10 {
            let block = append_node(&mut nodes, json!({"value":7777}));
            let index = append_node(&mut nodes, json!({"value":slot}));
            cuts.push(append_node(
                &mut nodes,
                json!({"func":"Get", "args":[block,index]}),
            ));
        }
        let sum = append_node(&mut nodes, json!({"func":"Add", "args":cuts}));
        let zero = append_node(&mut nodes, json!({"value":0}));
        let root = append_node(&mut nodes, json!({"func":"Add", "args":[sum,zero]}));
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        assert_eq!(
            program.region(root).unwrap().cut_nodes.len(),
            INLINE_REGION_INPUTS + 2
        );

        let mut expected = crate::runtime::WatchVm::new(&nodes);
        let mut actual = crate::runtime::WatchVm::new(&nodes);
        for slot in 0..10 {
            let value = slot as f64 * 1.25 - 3.0;
            expected.memory.set(7777, slot, value);
            actual.memory.set(7777, slot, value);
        }
        let expected_result = expected.execute(root).unwrap();
        actual.set_sono_gcc_program(Some(Arc::new(program)));
        let actual_result = actual.execute(root).unwrap();
        assert_eq!(actual_result.to_bits(), expected_result.to_bits());
        assert_eq!(actual.evaluation_count(), expected.evaluation_count());
        assert_eq!(actual.function_counts, expected.function_counts);
        assert_eq!(actual.sono_gcc_overflow_input_regions(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn and_negative_zero_stays_on_the_interpreter_path() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":7777}, {"value":0}, {"func":"Get", "args":[0,1]},
            {"value":1}, {"func":"Get", "args":[0,3]},
            {"func":"Add", "args":[1,3]}, {"func":"Negate", "args":[5]},
            {"func":"Negate", "args":[6]}, {"func":"Negate", "args":[7]},
            {"func":"Negate", "args":[8]}, {"func":"And", "args":[2,4]}
        ]))
        .unwrap();
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        assert!(program.region(9).is_some());
        assert!(program.region(10).is_none());
        let mut vm = crate::runtime::WatchVm::new(&nodes);
        vm.memory.set(7777, 0, -0.0);
        vm.memory.set(7777, 1, 1.0);
        let expected = vm.execute(10).unwrap();
        let mut gcc_mode = crate::runtime::WatchVm::new(&nodes);
        gcc_mode.memory = vm.memory.clone();
        gcc_mode.set_sono_gcc_program(Some(Arc::new(program)));
        assert_eq!(gcc_mode.execute(10).unwrap().to_bits(), expected.to_bits());
    }

    #[cfg(windows)]
    #[test]
    fn native_regions_read_special_and_out_of_range_memory_through_watch_vm() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":3000}, {"value":0}, {"func":"Get", "args":[0,1]},
            {"value":1000000}, {"func":"Get", "args":[0,3]},
            {"value":7777}, {"value":1000000}, {"func":"Get", "args":[5,6]},
            {"value":1004}, {"value":0}, {"func":"Get", "args":[8,9]},
            {"func":"Add", "args":[2,4,7,10]},
            {"func":"Negate", "args":[11]}, {"func":"Negate", "args":[12]},
            {"func":"Negate", "args":[13]}, {"func":"Negate", "args":[14]}
        ]))
        .unwrap();
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        assert!(program.region(15).is_some());

        let mut vm = crate::runtime::WatchVm::new(&nodes);
        vm.context.engine_rom = Arc::new(vec![12.5]);
        *vm.context.runtime_background.write().unwrap() = [-0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let expected = vm.execute(15).unwrap();
        let expected_counts = vm.function_counts.clone();
        let expected_evaluations = vm.evaluation_count();

        let mut gcc_mode = crate::runtime::WatchVm::new(&nodes);
        gcc_mode.context.engine_rom = vm.context.engine_rom.clone();
        gcc_mode.context.runtime_background = vm.context.runtime_background.clone();
        gcc_mode.set_sono_gcc_program(Some(Arc::new(program)));
        let actual = gcc_mode.execute(15).unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits());
        assert_eq!(gcc_mode.function_counts, expected_counts);
        assert_eq!(gcc_mode.evaluation_count(), expected_evaluations);
        assert_eq!(gcc_mode.sono_gcc_regions_executed(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn native_regions_use_watch_vm_for_dynamic_get_addresses() {
        let mut nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":7777}, {"value":0}, {"value":1},
            {"func":"Add", "args":[0,1]},
            {"func":"Add", "args":[1,2]},
            {"func":"Get", "args":[3,4]},
            {"value":2}, {"func":"Add", "args":[5,6]}
        ]))
        .unwrap();
        let root = append_node(&mut nodes, json!({"func":"Add", "args":[7,2]}));
        let (program, _, _, _) = SonoGccProgram::compile(&nodes).unwrap();
        assert!(program.region(root).is_some());

        let mut vm = crate::runtime::WatchVm::new(&nodes);
        vm.memory.set(7777, 1, 4.25);
        let expected = vm.execute(root).unwrap();
        let expected_counts = vm.function_counts.clone();
        let expected_evaluations = vm.evaluation_count();

        let mut gcc = crate::runtime::WatchVm::new(&nodes);
        gcc.memory = vm.memory.clone();
        gcc.set_sono_gcc_program(Some(Arc::new(program)));
        let actual = gcc.execute(root).unwrap();
        assert_eq!(actual.to_bits(), expected.to_bits());
        assert_eq!(gcc.function_counts, expected_counts);
        assert_eq!(gcc.evaluation_count(), expected_evaluations);
        assert_eq!(gcc.sono_gcc_regions_executed(), 1);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_compilation_reports_safe_interpreter_fallback() {
        let nodes: Vec<EngineNode> = serde_json::from_value(json!([
            {"value":1}, {"value":2}, {"func":"Add", "args":[0,1]}
        ]))
        .unwrap();
        assert!(SonoGccProgram::compile(&nodes)
            .err()
            .is_some_and(|error| error.to_string().contains("only on Windows")));
    }
}
