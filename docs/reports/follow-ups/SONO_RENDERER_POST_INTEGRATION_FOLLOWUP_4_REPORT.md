# Sono-Renderer Post-Integration Follow-up #4

## Engine / project compatibility analyzer

Implemented an evidence-oriented analyzer while preserving the historical
`compatibility ENGINE RESOURCES LEVEL` command and its legacy JSON fields. The
versioned report adds category assessments, operation findings, structural
findings, detailed selected-resource status, and an optional bounded dynamic
result. It uses categorical evidence, not a compatibility percentage.

## Architecture and evidence sources

- `src/compatibility.rs` owns the report schema, static analysis, resource
  checks, dynamic probe, and human-readable formatter.
- `src/runtime.rs` provides a conservative Watch operation capability
  classifier: implemented, explicitly unsupported, or unknown. Static graph
  presence is not treated as proof of reachability or execution.
- `src/sono_gcc.rs::inspect_operation_coverage` uses the compiler's eligibility
  classifier and reports per-node native coverage. Ineligible paths are
  described as VM-routed; that does not assert that the VM supports them.
- The compiler eligibility walk now observes its existing 256-level analysis
  bound. Deep or unsafe regions fall back instead of recursively exhausting the
  process stack.
- Existing format/resource loaders verify selected default skin, background,
  effect, and particle assets. The analyzer separates a missing manifest from
  a present manifest that is malformed or fails validation. Valid effect data
  with absent optional clip-audio payloads is reported as `optional_missing`,
  not malformed. SCP content-reference integrity findings are included.
- Additional graph checks report invalid node references, invalid constant
  values, bad callback targets, and invalid archetype declarations. When the
  optional dynamic probe runs, `WatchRuntime::new` validates full
  level/archetype/import binding without invoking callbacks.

## Status and fallback policy

The report supports `supported`, `unsupported`, `partial`, `fallback_required`,
`dynamically_verified`, and `unknown_runtime_dependent`. A concrete absent or
malformed required asset and a structurally rejected graph are reported as
unsupported in their relevant category. An unsupported or unknown operation
that merely appears in the graph makes Watch support partial, with a warning
that reachability and execution were not established. Integrity failures in
unrelated optional SCP resources are reported as partial and are not promoted
to a required-project-resource failure. No global compatibility verdict is
produced.

Sono-GCC coverage reports how many operation nodes are eligible and how many
are routed through Sono VM. On non-Windows targets, the analyzer states that
the current native DLL implementation is unavailable. On Windows it reports
the compiler backend identity but does not claim that rustc/toolchain loading
will succeed. It does not compile or mutate cache state.

## Bounded dynamic validation

`--dynamic` binds engine ROM, configuration defaults, and selected skin/effect/
particle names from the supplied SCP, then runs one single-threaded Watch
preprocess pass. It does not run gameplay frames, initialize/update/terminate
callbacks, render, compile native code, mix audio, or export.

The probe skips inputs above 100,000 Watch nodes or 64 level entities, limits
each callback to 100,000 VM evaluations, and caps runtime entities including
spawns at 4,096. It records the exact exercised scope and reports failures
without turning them into claims about unexercised operations. The full
Horizon and Next Sekai fixtures exceed the 64-entity probe cap, so the dynamic
option correctly reported `skipped_safety_limit` for both. Separate real
`run-watch` executions below exercised those complete levels.

## CLI and GUI

The existing `compatibility` command still emits JSON by default. Add
`--human-readable` for a concise summary, `--json` to state the structured
output preference explicitly, and `--dynamic` for the bounded preprocess
probe. Existing engine/level/resource inspectors, project validation, and
`run-watch` commands remain available.

The main GUI now has a Compatibility tab for the currently selected engine,
resource package, and level. It shows expandable category evidence, warnings,
fallbacks, limitations, operation/compiler coverage, selected-resource
validation, structural findings, and optional dynamic-probe scope.

CLI dispatch was moved out of `main` into `run_cli`; a fresh debug executable
previously exhausted its startup stack while building the large CLI command
match. After the split, both `renderer --help` and real compatibility commands
run normally.

## Tests and real-engine audit

Focused tests cover supported/unsupported/unknown classification, structural
node references, GCC-ineligible but VM-supported `Draw` routing, missing and
malformed resource manifests, unknown operation handling, dynamic execution
failure, and safety-limit skips. CLI parsing covers `--dynamic` and
`--human-readable`.

`cargo fmt --all -- --check`, `cargo check --all-targets`, ten compatibility
unit tests, the Sono-GCC VM-cut classifier test, and the CLI compatibility
parse test passed. Existing deprecation/dead-code warnings remain outside this
task.

Two supplied real projects were statically inspected and then run through
single-threaded Sono VM `run-watch` at time 0 with their supplied resources:

| Fixture | Static analyzer | Runtime cross-check |
|---|---|---|
| Horizon / Dreamer | 1,583 level entities; 47 distinct graph operation names; 2,128 Watch nodes; 4 default resources validated; zero structural findings and zero SCP integrity issues | 1,583 / 1,583 entities resolved; 2,229 runtime entities; 6,008 callbacks; 166,249 VM evaluations; 42 distinct executed operation kinds, all classified VM-supported; frame 0.233 s |
| Next Sekai / Larp 64x | 202 level entities; 73 distinct graph operation names; 4 default resources validated; zero structural findings and zero SCP integrity issues | 202 / 202 entities resolved; 647 runtime entities; 1,361 callbacks; 1,425,645 VM evaluations; 61 distinct executed operation kinds, all classified VM-supported; frame 1.125 s |

The timings are single local diagnostic runs, not benchmark claims. The
operation cross-check confirms the functions that actually executed were
classified as VM-supported; it does not prove every graph node executes or
establish full Sonolus equivalence. Sono-GCC native execution, GPU/FFmpeg
availability, render appearance, and real-time audio remain unverified by this
analyzer.

## Reproduction

From the repository root:

```powershell
renderer compatibility "TestingSuite\Horizon\Horizon.zip" "TestingSuite\Horizon\project(1).scp" "TestingSuite\Horizon\dreamer.json.gz" --json
renderer run-watch "TestingSuite\Horizon\Horizon.zip" "TestingSuite\Horizon\dreamer.json.gz" --resources "TestingSuite\Horizon\project(1).scp" --time 0 --single-threaded

renderer compatibility "TestingSuite\Next Sekai Engine\engine\Next Sekai Engine.zip" "TestingSuite\Next Sekai Engine\levels\larp 64x\ProSeka Faithful 0.8.4.scp" "TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.json.gz" --json
renderer run-watch "TestingSuite\Next Sekai Engine\engine\Next Sekai Engine.zip" "TestingSuite\Next Sekai Engine\levels\larp 64x\larp 64x.json.gz" --resources "TestingSuite\Next Sekai Engine\levels\larp 64x\ProSeka Faithful 0.8.4.scp" --time 0 --single-threaded
```

See [the compatibility analyzer guide](../../COMPATIBILITY_ANALYZER.md) for
status semantics, flags, and dynamic safety limits.
