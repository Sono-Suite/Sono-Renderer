# Visual, audio, and semantic regression laboratory

The regression laboratory compares bounded captures from the real Sono-Renderer
pipeline. It is opt-in: ordinary preview and export paths do not write
regression artifacts or hash SFX PCM. A capture enables the existing render
profiler and preserves its output next to the comparison report.

## What is compared

Each checkpoint uses `RenderConfig::render_frames`, the same Watch session,
skin/resource loading, gameplay UI, watermark, CPU/WGPU backend, and compositor
used by the renderer. The versioned reference stores independent layers:

- runtime state and callback observations;
- Watch operation counts and Sono-GCC native coverage;
- ordered display list / Draw data;
- audio events and scheduled effects;
- particle events;
- the gameplay UI adapter state and deterministic watermark pose;
- lossless RGB PNGs and their RGB SHA-1;
- optional checkpoint stage timings.

`--include-media` adds a finite video/audio pass of at most 120 seconds. It
records the SFX event-sequence hash, pre-AAC mixed SFX PCM hash, BGM sample
window/timeline, and selected stable FFprobe stream fields. Encoded MP4 bytes
are deliberately not hashed: encoder/container metadata can vary without a
semantic audio or visual change. The media pass uses temporary output and does
not keep an MP4. For BGM PCM comparison, configure music and enable the BGM
layer. Media capture is per execution-mode/resolution variant, so avoid pairing
it with a large VM/GCC and resolution matrix unless that work is intentional.

Pixel comparison is exact by default. `--pixel-tolerance` may be set to 1 or 2
per channel for known small rounding variance; values above 2 are rejected.
Only pixel failures produce a difference image and an amplified heat map. JSON
layers report changed field paths. Floating-point values in semantic JSON
layers are canonicalized to 1e-9 absolute precision to suppress final-bit
evaluation noise; integer values, strings, arrays, and larger numeric changes
remain exact. Pixel comparison remains independent and exact by default.
Timing uses the median of captured samples;
it is called changed only when both the absolute difference exceeds 2 ms and
the relative difference is at least 5%. Timing does not determine pixel or
semantic equality by itself.

`checkpoint_step_ms` is the profiled wall duration from the preceding selected
checkpoint (or from the session's initial pre-roll to the first checkpoint),
including Watch advancement and that checkpoint's render/compositor work. It is
not an isolated single-frame cost. The stage fields are attribution within
that step and may overlap; do not add them to reconstruct elapsed time.

## Workload identity and schema

References use schema version 1. The manifest records the engine, resources,
level, optional music content fingerprints; normalized effective render/UI/
watermark configuration; FPS, start/duration, backend, gameplay UI mode,
resolution set, checkpoint times, requested timing sample count, renderer
version/optional commit, OS, architecture, and capture timestamp. Paths are
normalized through content fingerprints for input assets, so relocation alone
does not make a workload different. The timestamp is metadata, never an
elapsed duration.

Different workload fingerprints/configuration are reported as **not
comparable**; they are not classified as a regression. Environment changes
are disclosed separately. Missing modes/checkpoints, invalid schema, absent
PNGs, and hash/dimension mismatches fail closed. VM/GCC comparison captures
both paths from the same config; interpreter remains the semantic reference.
If GCC did not execute native regions at a checkpoint, parity is reported as
unavailable and the command returns the not-comparable status rather than
silently declaring parity.

Create or replace a reference only with the explicit `update` command. A new
reference is rendered into a staging directory and atomically published. A
replacement requires `--replace`, retains the old bundle until publication
succeeds, and prints a layer-by-layer summary of observed changes. Review the
PNGs and manifest before treating an updated reference as expected behavior;
there is no automatic blessing after a test failure.

## CLI workflow

The repository includes `regression-fixtures/next-rush-larp64x.json`. Its
inputs are the local Larp 64x files under the ignored `TestingSuite/` tree, so
that fixture is available only in a checkout with those local test assets.
The example profiles use the existing asset-backed HUD and validation
watermark. Run from the repository root:

```powershell
cargo run --release -- regression update regression-fixtures/next-rush-larp64x.json `
  --reference artifacts/regression-lab/references/next-rush-larp64x `
  --fixture next-rush-larp64x `
  --checkpoint 24.667 --checkpoint 24.833 `
  --resolution 1280x720 --resolution 1024x768 `
  --ui-profile ui-profiles/default.json `
  --watermark-profile watermark-profiles/validation-text.json `
  --compare-vm-gcc --timing-samples 3

cargo run --release -- regression inspect artifacts/regression-lab/references/next-rush-larp64x

cargo run --release -- regression run regression-fixtures/next-rush-larp64x.json `
  --reference artifacts/regression-lab/references/next-rush-larp64x `
  --ui-profile ui-profiles/default.json `
  --watermark-profile watermark-profiles/validation-text.json
```

The two resolutions exercise landscape 16:9 and 4:3 layout. The reference
stores the reference PNGs under descriptive mode/resolution directories. A
run writes current PNGs and `comparison.json` under
`artifacts/regression-lab/runs/`; mismatches also write bounded `diffs/*-diff.png`
and `*-heatmap.png`. Supply `--output DIRECTORY` to retain a run elsewhere;
the target must not already exist. Use `--json` on any subcommand for
machine-readable output. `run` exits 0 for a comparable pass, 1 for a layer
mismatch, and 2 when comparison/parity is not comparable. Capture/setup errors
are reported as command errors.

To record a separate audio/media reference without multiplying full video
exports across the VM/GCC matrix, use one mode and one resolution:

```powershell
cargo run --release -- regression update regression-fixtures/next-rush-larp64x-audio.json `
  --reference artifacts/regression-lab/references/next-rush-larp64x-media `
  --fixture next-rush-larp64x-media --checkpoint 2.5 `
  --resolution 16x16 --ui-profile ui-profiles/default.json `
  --watermark-profile watermark-profiles/validation-text.json `
  --include-media --timing-samples 1
```

The audio fixture is intentionally a one-second segment around scheduled
effects. A 10-second full-resolution CPU media capture was much slower on the
available machine; short low-resolution integration output still exercises
real Watch, BGM sample-window, SFX mixing, FFmpeg, and FFprobe paths without
retaining a large export.

`regression run` reuses the checkpoint schedule, resolution, modes, sample
count, and media setting from the reference. Reference replacement is explicit:
add `--replace` only after reviewing the previous capture and the printed
changed-layer summary.

## Desktop GUI and artifact retention

The desktop **Regression Lab** tab accepts a config, reference directory,
checkpoint, sample count, optional VM/GCC parity, and optional media capture.
“Update reference...” is an explicit action with a confirmation when the
target exists. It does not add UI/profile configuration to normal rendering
controls. The GUI uses the same regression module and reference schema as the
CLI.

Automatically-created run bundles are marked and capped at the newest 20
directories under `artifacts/regression-lab/runs`. A custom `--output`
directory is user-managed and is not pruned. References are never automatically
deleted or replaced. PNG, resolution, checkpoint, sample, and media-duration
bounds prevent accidental unbounded captures; reports store small metric JSON
beside images rather than embedding binary artifacts.

## Tests and limitations

Regression unit tests cover schema/version gates, corrupt references, input
identity, JSON-layer changes, pixel tolerance/statistics, diff artifacts,
timing noise policy, pre-encoding PCM changes, and explicit reference
replacement. CLI parsing tests cover repeated checkpoints/resolutions and
profile options. A real fixture run remains an explicit developer/CI step
because the included `TestingSuite/` inputs are local and ignored. No hard
wall-time threshold is placed in correctness tests.

The comparison is deterministic only to the extent of the selected backend,
engine/resources, external profiles/data, fonts/assets, and CPU/runtime path.
WGPU adapter changes are disclosed; they can affect raster pixels. The capture
does not claim to recreate a Sonolus client's authoritative judgement results:
it records the renderer's supplied runtime/event state. BGM comparison records
sample-window/timeline metadata, not a canonical hash of decoded BGM samples.
SFX PCM capture is pre-AAC and profiling-only; there is no encoded-container
byte equality assertion.
