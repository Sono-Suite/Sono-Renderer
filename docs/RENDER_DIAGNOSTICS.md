# Render diagnostics

Render diagnostics explain a completed or failed render using the existing
`RenderPerformanceRecord`, job state, and output validation facts. The analyzer
does not start clocks. It classifies existing measurements and marks missing
evidence as unavailable.

## Where records live

- A completed direct export returns a diagnostic snapshot alongside its
  performance record. The report is atomically stored under the performance
  history's `diagnostics` directory. This sidecar keeps output-validation facts
  that do not belong in the timing record.
- A queued job stores its diagnostic snapshot in the persistent job record.
  Failed and cancelled jobs also retain a structured report with the phase,
  failure category, and sanitized error chain. The bounded sidecar store also
  retains successful job reports for up to 500 records.
- Failed direct exports are persisted as diagnostic-only records; they do not
  claim completed-render timing data.
- The diagnostic snapshot is small metadata. Frame dumps, audio, video, and
  verbose logs are not copied into it.
- Job-store records written before this schema remain readable: the new
  diagnostic field defaults to absent. Diagnostic JSON declares schema version
  1 and rejects unsupported versions in the CLI and desktop view.
- Diagnostic sidecars use one atomic JSON file per record and prune oldest
  files beyond the 500-record retention limit.

Exports sanitize absolute paths and credential-like values. The report keeps
workload fingerprints, renderer configuration identity, resolution, frame
rate, execution mode, and effective backend classification so comparisons can
be tied to the configuration that ran without copying source paths.

## Timing and conclusions

The disjoint `ExclusiveWallSpan` stages partition the render execution wall
time. Diagnostics call the largest of these the **largest measured phase**;
that identifies where the wall time was observed but does not by itself prove a
root cause.

`OverlappingAttribution` and `PipelineOverlap` values remain separate. Their
Watch, producer, render, output, queue-residence, and wait spans can overlap and
must not be added to the exclusive phase totals. The current telemetry does not
contain a complete dependency graph, so the exact asynchronous critical path
and the cause of a particular wait are explicitly reported as unavailable.

Sono-GCC cache hits and fallbacks are observations from the renderer's actual
runtime/cache result. A hit does not imply a claimed amount of time saved; that
requires a comparable cold sample. A fallback is classified as VM execution
and is not presented as native Sono-GCC performance. Use matched repeated
benchmarks from the performance laboratory for VM/GCC conclusions.

Successful exports include selected FFprobe facts (stream count, codecs,
dimensions, frame rate, and duration) and whether FFprobe validation passed.
Absolute FFmpeg/FFprobe paths and the raw FFprobe payload are omitted. Detailed
GPU submit/wait/readback and encoder substage timing is shown only when the
existing opt-in detailed profiler captured it. No GPU timestamp query or extra
synchronization is introduced by diagnostics.

## Inspect and export

```text
renderer diagnostics record <diagnostic-id> [--json]
renderer diagnostics record <diagnostic-id> --output <report.json>
renderer diagnostics job <job-id> [--store <queue.json>] [--json]
renderer diagnostics job <job-id> --output <report.json>
renderer diagnostics performance <performance-record-id> [--json]
renderer diagnostics performance <performance-record-id> --output <report.json>
```

The desktop **Render diagnostics** tab selects a persisted job, shows facts,
observed events, derived conclusions, recommendations, and unavailable
evidence separately, and exports the same versioned JSON report. It shows
advanced stage and pipeline data without merging overlapping spans.

For a failed direct CLI render, the CLI prints a sanitized structured failure
report to stderr. Failed queued jobs keep that report in the queue store for
later inspection. No diagnostic-specific timing history is maintained; the
job and performance stores remain the existing sources of truth.
