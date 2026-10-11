# Performance laboratory

Sono-Renderer writes one versioned `RenderPerformanceRecord` for each
successful render. The same record is attached to queued job outcomes, shown
in the desktop Performance / benchmarks tab, and consumed by the CLI history
and comparison commands. Timing consumers should not add their own stopwatches
or infer durations from job log timestamps.

## Timing semantics

Elapsed values come from monotonic `Instant` clocks. UTC millisecond values in
records are timestamps only. The top-level `stage_timings` rows use
`exclusive_wall_span`: `Preparing`, optional `Validating`, `Rendering`,
`AudioMixing`, `AudioEncoding`, `Finalizing`, and a small export bookkeeping
residual. These rows are disjoint and their sum tracks `execution_time_ms`.

`overlapping_attribution` rows are the existing opt-in per-frame profiler
aggregates. They can be nested (for example callback time within Watch time)
or concurrent (worker time across frames); they are diagnostic attribution,
not additive stages. The GUI labels them separately. Pipeline producer, render,
queue, and output totals are stored in `pipeline_overlap` with a render wall
span. Producer-blocked time and queue residence are correlated; lane values
can overlap and are never included in the disjoint phase sum. No GPU timestamp
query or synchronization is added, so GPU elapsed execution is not currently
reported independently from submit/wait/readback spans.

For direct exports, execution covers the export path through staged output
publication. Queued jobs replace that value with the full worker execution
interval, including validation, staging, and final publication. `queue_wait`
is recorded separately from a monotonic in-process enqueue instant. For jobs
restored after a process restart, that instant cannot be reconstructed without
using wall time as a duration; queue wait is therefore absent (`null`) rather
than fabricated. The job's persisted UTC submitted/start/finish fields remain
timestamps.

## Sono-GCC and cache state

Records identify the requested Watch mode and the effective backend status.
When compilation data is available they include cache lookup, compilation,
native load, graph identity, disk validation, metadata reconstruction, and
cache write durations. The cache state distinguishes `miss_compiled`,
`process_hit`, `persistent_hit`, `fallback`, and `unknown`. Sono VM remains the
correctness oracle.

The benchmark's first Sono-GCC full render is stored as an
`initial_cache_observation` and excluded from the warm distribution. It is a
cold sample only when that actual run reports a cache miss. The benchmark does
not clear or mutate the user's Sono-GCC cache to manufacture a cold result.
Warm-ups run before measured samples, which then alternate VM/GCC order to
reduce order bias. The report keeps cache observations and every measured
sample separate, and compares complete-render medians rather than declaring a
winner from the compile stage alone.

When whole-chart boundary discovery is the first Watch pass, compiler metrics
are captured from that Watch-only session after discovery so a later streaming
cache hit does not hide the initial compile or cache lookup.

Compiler/cache substage durations occur inside the enclosing preparation or
validation phase. They are explanatory backend metrics and must not be added
to the disjoint phase spans.

The benchmark and history comparison also require evidence that native
Sono-GCC executed. A requested Sono-GCC run that fell back to the VM, or whose
effective backend could not be confirmed, is shown with its timings and status
but is not presented as a VM-versus-GCC performance comparison.

## Benchmarking and CLI

Use a renderer JSON configuration as the workload. Benchmark outputs are
written in a temporary directory and removed after the run; the configured
output path is not replaced.

```text
renderer performance benchmark config.json --warmup 1 --iterations 3
renderer performance benchmark config.json --compare-vm-gcc --warmup 1 --iterations 3 --output benchmark.json
renderer performance benchmark config.json --measure-telemetry-overhead --json
renderer performance history --limit 20
renderer performance history --json
renderer performance compare RECORD_ID_A RECORD_ID_B
renderer performance compare RECORD_ID_A RECORD_ID_B --json
```

Warm-ups are limited to 10; measured iterations are limited to 20 and default
to 3. P95 is shown only with at least 20 samples. The optional telemetry
overhead mode adds a matched set of runs with deep per-frame profiling enabled
and compares its median with the lightweight telemetry runs. This estimate
includes normal machine noise and should be repeated before drawing a
conclusion. It does not disable record creation, which is part of the normal
render path.

Record comparison checks engine, resource, level, and music fingerprints;
effective configuration; resolution; FPS; time range; pixel backend; renderer
build; and environment identity. Sono VM versus Sono-GCC is an intentional
comparison variable, as is the reported cache state. Unmatched inputs are
reported as `not_comparable`. For matching workloads, changes of at most 2 ms
or less than 5% are classified `within_noise`; larger changes are labeled
faster or slower, not as a statistically proven regression.

Use `--json` for machine-readable output. `--output` writes a new JSON report
atomically and refuses to overwrite an existing file. Detailed profile rows
are available in record JSON and from the GUI's advanced per-frame attribution
section.

## Persistence and retention

History is stored as one atomic JSON file per record, schema version 1, under:

- Windows: `%LOCALAPPDATA%\Sono-Renderer\performance`
- Linux/macOS: `$XDG_STATE_HOME/sono-renderer/performance`, falling back to
  `~/.local/state/sono-renderer/performance`

The store keeps the latest 500 records and contains metrics only; temporary
render outputs and large diagnostic artifacts are not retained. Source paths
are not stored. Input identity uses a SHA-1 fingerprint over canonical path,
file size, and modified-time metadata; the effective configuration is hashed
after removing output paths and notification credentials. No notification
secrets are recorded or fingerprinted. Renderer version, optional
`SONO_RENDERER_GIT_COMMIT`, OS, architecture, and available CPU/GPU identity
are recorded.

The desktop Performance / benchmarks tab shows recent history, the latest
comparison, phase spans, queue wait, cache state, and pipeline overlap. A
benchmark button uses the current project/configuration without changing the
user's output path. The latest benchmark report can be exported as JSON from
the GUI. The same history and run workflow are exposed by the CLI.

## Validation

Correctness tests validate schema round trips, unsupported-version handling,
atomic history/retention behavior, non-additive overlap semantics,
comparison identity and noise classification, and queue/execution separation.
The real export integration exercises VM and Sono-GCC jobs, queue wait,
CPU/WGPU frame paths, pipeline export, FFmpeg output, and persisted
performance-record references. These tests assert structure and parity, not
elapsed-time thresholds.
