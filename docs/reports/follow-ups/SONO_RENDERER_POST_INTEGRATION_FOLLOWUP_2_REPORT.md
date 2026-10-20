# Sono-Renderer Post-Integration Follow-up #2 - Performance Laboratory

## Result

Implemented one versioned `RenderPerformanceRecord` and wired it through direct
exports, queued `RenderJobOutcome`s, persistent history, the desktop
Performance laboratory, and the `renderer performance` CLI. Elapsed durations
come from monotonic clocks; UTC values remain timestamps. See
[`docs/PERFORMANCE_LAB.md`](../../PERFORMANCE_LAB.md) for the operator guide and
reproduction commands.

## Timing model

The record contains the complete execution duration, optional queue wait and
user latency, workload/environment identity, requested and effective Watch
backend, cache/compiler metrics, phase timings, optional pipeline overlap,
frame/runtime counters, and opt-in profile attribution.

Top-level export phases are disjoint wall spans: preparation, optional
validation, rendering, audio mixing/encoding, finalization, plus an explicit
bookkeeping residual. Sono-GCC compile/cache substage timings are explanatory
metrics inside preparation or validation and are not added to those phase
spans. The residual makes the phase partition track the measured execution
interval. Deep per-frame profiler rows are tagged
`overlapping_attribution`; they can nest or overlap and are not added to the
phase sum. Pipeline producer/render/output lane totals are similarly
non-additive and are displayed with their wall span and queue/wait indicators.
GPU timestamp queries are not issued, avoiding measurement synchronization.

Direct-render execution covers the export through staged output publication.
Queued execution covers the worker's validation, staging, export, output
publication, and job finalization. Queue residence is calculated from a
monotonic enqueue instant and kept separate. A restored job has no recoverable
monotonic enqueue instant, so its queue wait remains absent rather than being
derived from wall-clock subtraction.

The export phase boundary was corrected during the pass: FFmpeg startup and
streaming `FrameSession` initialization belong to preparation. They no longer
inflate rendering time. This matters on short exports where setup dominates.

## VM, Sono-GCC, and cache behavior

Each record captures actual compile/cache metrics when the runtime provides
them: cache lookup, compilation, native load, graph identity, disk validation,
metadata reconstruction, and cache write. Cache state distinguishes
`miss_compiled`, `process_hit`, `persistent_hit`, `fallback`, and `unknown`.
For whole-chart renders, metrics come from the earliest Watch-only discovery
session when it runs before streaming, so the later cache hit cannot obscure
initial compilation work.

Benchmarks preserve a first Sono-GCC cache observation separately from warm
samples, run configured warm-ups, alternate VM/GCC sample order, and compare
complete-render medians. They never clear the user's compiler cache. A
requested Sono-GCC run that fell back to VM, or whose native execution cannot
be confirmed, now has its timing retained but is marked not comparable to VM
versus native GCC. Historical comparisons use the same effective-backend
guard. The VM remains the correctness oracle.

## Benchmark surfaces and storage

The GUI Performance laboratory benchmarks the current renderer inputs, offers
warm-up/run counts, optional VM/GCC comparison and detailed-profile overhead
sampling, shows recent records and stage/pipeline details, compares the latest
records, and exports the latest benchmark JSON atomically. The CLI supports
`history`, `compare`, and repeated `benchmark` commands with human or JSON
output. Benchmark report output refuses to overwrite an existing file.

Small versioned JSON records are stored atomically, one file per render, with a
default 500-record retention cap. Temporary video outputs and large profiler
artifacts are not retained. The GUI and CLI consume the same store and record
schema. Filesystem paths are not stored; source identity fingerprints use
canonical path, size, and modification metadata. Configuration identity omits
output paths and notification credentials. OS, architecture, renderer build,
and available CPU identity are recorded; GPU identity is available when deep
profiling collected an adapter identity.

Comparisons require matching source/configuration fingerprints, resolution,
FPS, time range, render backend, build, environment, and direct-versus-queued
scope. VM/GCC and GCC cache state are intended comparison variables when
effective execution is confirmed. Changes no larger than 2 ms or 5% are
classified within noise; larger changes are labeled faster/slower, not as
statistically proven regressions. Benchmark distributions include min,
median, mean, and max; p95 is shown only for at least 20 samples. Defaults are
one warm-up and three measured runs, with limits of 10 and 20.

## Validation and representative measurements

Correctness checks do not impose timing thresholds. The performance unit
tests cover schema round-trip/version rejection, atomic persistence and
retention, phase versus overlapping-attribution semantics, queue/execution
field separation, workload comparability, fallback rejection, noise handling,
and distribution statistics. The real export/job integration exercises VM
exports, a Sono-GCC request and its effective backend status, CPU/WGPU export
paths, pipeline and FFmpeg output, queue wait, output publication, and the
benchmark telemetry-overhead workflow.

Validation performed:

- `cargo test --offline --all-targets`: passed (193 library tests, 18 CLI
  tests, remaining targets passed; 3 ignored).
- `cargo test --offline --lib single_watch_export_audio_pipeline_offsets_and_failure_cleanup -- --nocapture`:
  passed in 154.20 seconds, including a real export/job integration and the
  overhead measurement.
- `cargo test --offline --lib performance::tests`: 8 passed, including atomic
  benchmark-report output and native-GCC comparability checks.
- The two focused whole-chart discovery tests passed after the compiler-metric
  capture was moved to the end of discovery.
- Final-source `cargo check --offline --all-targets`, `cargo fmt --all --check`,
  and `git diff --check` passed.

The full all-target test run preceded the final small changes to shared
benchmark-report export and whole-chart metric capture. Those final changes
were compiled for all targets and covered by the focused tests listed above.

On the local test fixture, a queued 24-frame CPU VM render recorded 5618.98 ms
execution, 80.19 ms queue wait, 5159.39 ms preparation, and 167.20 ms
rendering. Its audio/finalization phases and queue residual bring the disjoint
phase sum to the execution duration within rounding. A paired 24-frame
Sono-GCC-requested job recorded 5702.00 ms execution and 95.15 ms queue wait,
but the engine had no compilable Watch regions and the runtime fell back to
Sono VM. The system therefore withholds a VM-versus-GCC conclusion for that
pair. These short-fixture timings are examples of stage attribution, not a
general performance claim.

Two real-render overhead checks each used one startup-dominated sample per
mode. One measured lightweight 4003.337 ms and detailed-profile 3858.772 ms
(-144.565 ms, -3.61%); the all-target run measured 3864.72 ms and 3989.72 ms
(+125.00 ms, +3.23%). Both were classified within noise and the direction
reversed, so this does not establish that profiling is free or faster.
Repeated measurements on a representative workload are needed for a useful
overhead estimate. Core phase telemetry stays enabled in both modes; this
comparison estimates the extra cost of detailed profiling.

## Limitations

- This environment's integration engine did not exercise native Sono-GCC
  compilation or a warm native cache hit. Its requested GCC path correctly
  reported VM fallback; native performance comparison remains unmeasured here.
- GPU execution time is not isolated because the renderer does not currently
  request GPU timestamps. Existing wall spans and overlapping pipeline lanes
  are still available, but they cannot attribute device execution separately.
- Source fingerprints use path/size/mtime metadata, not full input-content
  hashes. This avoids hashing large inputs on every render but can miss an
  in-place change that preserves all metadata.
- The overhead sample count was intentionally one for a bounded integration
  run. Use several measured iterations and, where appropriate, at least 20 for
  p95 before drawing conclusions.

## Reproduction

```text
renderer performance history --limit 20
renderer performance compare RECORD_ID_A RECORD_ID_B --json
renderer performance benchmark config.json --warmup 1 --iterations 3
renderer performance benchmark config.json --compare-vm-gcc --warmup 1 --iterations 3 --measure-telemetry-overhead --output benchmark.json
```

The desktop app exposes the same benchmark workflow in its Performance /
benchmarks tab using the current configuration snapshot.
