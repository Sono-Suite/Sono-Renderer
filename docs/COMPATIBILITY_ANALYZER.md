# Engine / project compatibility analyzer

The compatibility analyzer extends the historical `renderer compatibility`
JSON report. Its original `engine`, `level_entity_count`, `level_archetypes`,
`resources`, and resource-integrity fields remain available. Schema version 1
added category assessments, per-function implementation and compiler
coverage, structural findings, and an optional bounded dynamic result.
Schema version 2 adds `approximate` to the per-operation VM implementation
status vocabulary; legacy report fields and their names remain available.

## Run it

```text
renderer compatibility ENGINE.zip RESOURCES.scp LEVEL.data
renderer compatibility ENGINE.zip RESOURCES.scp LEVEL.data --human-readable
renderer compatibility ENGINE.zip RESOURCES.scp LEVEL.data --dynamic --json
```

No flags preserves the historical machine-readable JSON output. `--json` is
explicit machine-readable output; `--human-readable` prints the summary and
operation inventory. Existing `inspect-engine`, `inspect-level`,
`inspect-resources`, `validate-project`, `compile-engine`, and `run-watch`
commands remain unchanged.

The desktop Compatibility tab analyzes the currently selected engine,
resources, and level. Its optional dynamic checkbox uses the same bounded probe
as `--dynamic`.

## Evidence and status meanings

Each category provides evidence, warnings, fallback behavior, and limitations.
The status vocabulary is:

- **Supported**: the package was parsed and the stated static capability is
  present in this build. It is not a promise that every callback executes.
- **Unsupported**: a concrete required input is absent/malformed, or static
  structure contains an error that the current host rejects.
- **Partial**: a known implementation gap or an operation of uncertain runtime
  reachability affects this category.
- **Fallback required**: Sono-GCC does not cover all relevant nodes or is
  unavailable on the current target; execution is routed through Sono VM
  where that operation is implemented.
- **Approximate** (operation-level status): the VM executes a bounded,
  documented compatibility choice where public semantics are incomplete.
  Approximate is distinct from `Unsupported`; a Watch VM category containing
  approximate operations is reported as **Partial**, not fully supported.
  Runtime execution emits one warning per operation family for the job, and
  strict policy can reject such an operation before evaluating its arguments.
- **Dynamically verified (exercised scope only)**: the named bounded operation
  completed. This is never shorthand for full engine compatibility.
- **Unknown / runtime dependent**: static evidence cannot settle the question.

The Watch VM capability registry lives in `src/runtime.rs` beside the VM
dispatcher. It distinguishes known implemented operations, known missing
operations with compatibility limitations, and names outside the registry.
Approximate operation records include their GCC fallback count and never
increase direct-native eligibility. A function appearing in a graph only
records static presence. It does not establish reachability, branch
selection, argument validity, or execution. GCC per-node coverage is produced
by `sono_gcc::inspect_operation_coverage`, which calls the same eligibility
classifier used during compilation. The analyzer does not compile code or
change cache state.

The analyzer validates engine metadata default resources by loading the
selected skin, background, effects, and particle data through the renderer's
existing decoders. It separates missing manifests from present-but-malformed
manifests and reports content-addressed reference integrity issues. Runtime
selected IDs that are not represented by engine defaults remain unknown.
An effect resource whose data is valid but whose optional clip audio is absent
is reported as `optional_missing`; that does not make the effect manifest
malformed.

The category list includes a **Chart Studio** breakdown for raw LevelData
support, this document's in-memory parse/serialize round-trip, authoring-schema
availability, high-level editor availability, and preview. These dimensions
remain separate from Watch VM and renderer compatibility. The analyzer does
not accept/load a Chart Studio schema file, so schema availability is marked
unknown and the high-level path is reported as partial; Generic / Raw Mode is
the fallback. A matching schema can add declared authoring controls in Chart
Studio; it does not make those declarations Sonolus runtime requirements or
prove gameplay semantics. The parse/serialize comparison is not byte identity
and is not the standalone gzip export/reimport test. Level archetype
declarations are read from `EnginePlayData`; `EngineWatchData` archetypes and
callbacks are reported as runtime data and do not substitute for missing
PlayData declarations.

## Bounded dynamic probe

`--dynamic` uses `WatchRuntime::new`, binds the engine ROM, option/UI defaults,
and selected skin/effect/particle names from the SCP, forces single-threaded
execution, and runs the Watch preprocess system once. It does not render a frame, invoke
update/initialize/terminate callbacks, run Sono-GCC, perform audio mixing, or
export media. The report lists these exact steps.

The probe skips graphs above 100,000 nodes or levels above 64 entities. It caps
each callback at 100,000 VM evaluations and limits runtime entities (including
spawns) to 4,096. A runtime error is recorded as an execution failure for that
probe; it does not convert static unknown operations into a global
incompatibility verdict. The regular VM operation/depth guards remain active.

## Limits

Static analysis does not infer graph reachability, produce gameplay state, or
prove render appearance. Format versions are reported as metadata but are not
mistaken for compiler/runtime versions or checked against a fabricated support
range. GPU adapter availability, FFmpeg, filesystem output, and real-time audio
playback are not probed here. For behavior outside the bounded preprocess
scope, use a real render or a purpose-built runtime fixture.
