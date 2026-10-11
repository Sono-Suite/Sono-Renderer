# Sono-Renderer Post-Integration Follow-up #3

## Deep render diagnostics / performance explanation

Implemented a structured diagnostic layer that consumes the existing
`RenderPerformanceRecord` instead of adding timers or a second performance
model.

### Architecture

- Added versioned `RenderDiagnosticRecord` with separate finding kinds for
  measured facts, observed events, derived conclusions, recommendations, and
  unavailable evidence.
- Successful direct exports return the diagnostic with their `ExportReport`.
  Queued jobs persist it beside the job state; completed records snapshot the
  canonical performance record, while failed/cancelled attempts retain phase,
  failure kind, subsystem classification, and a sanitized cause chain.
- A bounded, atomic diagnostic sidecar store preserves successful direct
  export output-validation facts and direct-render failures. It retains at most
  500 small JSON records; it does not contain media, frame dumps, or raw logs.
- Existing queue wait, execution duration, phase spans, pipeline overlap, VM/GCC
  state, and compiler/cache observations are reused without new clocks.
- Diagnostics derive the largest disjoint phase, but do not call it an exact
  asynchronous critical path. Overlapping lane and per-frame measurements are
  shown separately and never summed into phase totals.
- Successful export reports include a minimal FFprobe validation summary,
  selected codec/stream facts, FFmpeg installation source, and optional
  deterministic-frame validation. Raw FFprobe JSON and absolute tool paths are
  omitted.
- Absolute paths and credential-like values are redacted in diagnostic error
  chains and status text. Diagnostic JSON is atomically exported to a new path
  and carries schema version 1.

### Frontends

- The desktop GUI has a **Render diagnostics** tab for persisted jobs and the
  latest direct render. It separates facts from conclusions and unavailable
  evidence, and exposes detailed phase, overlap, cache, GPU, and encoder data
  in an advanced section.
- CLI commands:

  ```text
  renderer diagnostics record <diagnostic-id> [--json]
  renderer diagnostics record <diagnostic-id> --output <report.json>
  renderer diagnostics job <job-id> [--store <queue.json>] [--json]
  renderer diagnostics job <job-id> --output <report.json>
  renderer diagnostics performance <record-id> [--json]
  renderer diagnostics performance <record-id> --output <report.json>
  ```

- Direct CLI failures print a sanitized structured diagnostic to stderr.
- Job and performance stores remain authoritative. Diagnostics do not add a
  separate history store or copy large render artifacts.

### Evidence and limitations

- Cache-hit observations do not claim time saved without a comparable cold
  sample. VM/GCC conclusions are deferred to matched repeated benchmarks.
- Fallback runs are identified as VM execution and excluded from native-GCC
  claims.
- Producer-blocked, queue-residence, consumer-waiting, render-lane, and output-
  lane metrics are reported as overlapping attribution. The dependency graph
  needed for an exact async critical path is not available in current telemetry.
- Detailed GPU wait/map/readback and encoder substage attribution remains
  opt-in because it comes from the existing deep profiler. No GPU execution
  timestamp or synchronization was added.
- For failed renders, no completed `RenderPerformanceRecord` exists, so stage
  durations remain unavailable; lifecycle phase and error context are retained.

See [the render diagnostics guide](docs/RENDER_DIAGNOSTICS.md) and
[the performance laboratory guide](docs/PERFORMANCE_LAB.md).

### Verification

Passed in this work session:

- `cargo check --all-targets`
- `cargo fmt --all -- --check`
- `cargo test render_diagnostics::tests`: 7 passed, including schema handling,
  path/credential redaction, exclusive-versus-overlapping timing semantics,
  queue-wait separation, FFprobe summary extraction, cache-hit/fallback
  wording, missing metrics, and bounded atomic storage.
- `cargo test --bin renderer performance_history_compare_and_benchmark_commands_parse`:
  CLI command parsing passed.
- `cargo test --lib scheduler_selects_priority_but_ages_old_low_priority_jobs`:
  persisted job compatibility with the new optional diagnostic field passed.
- `cargo test --lib video_export::tests::single_watch_export_audio_pipeline_offsets_and_failure_cleanup`:
  real CPU and WGPU export variants passed in 156.64 s. The test exercised
  frame pipelines, FFmpeg muxing, FFprobe validation, determinism variants,
  and the new output facts attached to the authoritative performance ID.

Compilation continues to report existing unrelated deprecation/dead-code
warnings in runtime, profiling, and audio-prepass code. This pass did not run
the full repository test suite or a new repeated VM/GCC benchmark. It also did
not quantify the diagnostic formatter separately; it runs after export, adds
no timers or GPU synchronization, and copies/analyzes already collected data.
