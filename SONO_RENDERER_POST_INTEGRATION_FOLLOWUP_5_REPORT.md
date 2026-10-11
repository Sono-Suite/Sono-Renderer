# Sono-Renderer Post-Integration Follow-up #5

## Visual / audio / semantic regression laboratory

Implemented an opt-in regression path that captures real renderer output and
compares visual, audio, runtime, and renderer-state layers independently. The
existing render path remains unchanged when capture is off. The full
user-facing procedure, commands, layer semantics, and limits are documented in
[`docs/REGRESSION_LAB.md`](docs/REGRESSION_LAB.md).

## Architecture

- `src/regression.rs` owns the single schema, workload identity, capture,
  comparison, retention, diff-image creation, and reference-update policy.
- Capture calls the real `RenderConfig::render_frames` pipeline with profiling
  enabled only for an explicit regression operation. Multiple checkpoints are
  rendered in one ordered Watch session per mode/resolution.
- `RenderJob` and normal render histories are not given a second timing schema;
  this lab records bounded checkpoint-step attribution in the capture manifest.
- `src/audio.rs` computes a pre-AAC SFX PCM hash only when the media regression
  opts in. Ordinary SFX mixing leaves PCM hashing disabled.
- The existing GUI has a **Regression Lab** tab; CLI commands are
  `regression update`, `regression run`, and `regression inspect`.

Each frame capture stores runtime semantics, operation counts, the display
list, audio and particle events, gameplay UI adapter state, watermark pose,
lossless RGB PNG, and per-stage profiler values. Optional media capture stores
SFX event and PCM hashes, BGM timeline/sample-window metadata, and selected
FFprobe stream information. It does not preserve an MP4 or compare compressed
container bytes.

The comparison has separate JSON-layer and pixel results. Pixel mismatch
reports changed pixel count/ratio, max and mean channel delta, and a bounding
box; only mismatches produce an RGB difference and heatmap PNG. Workload
fingerprint/configuration mismatch is **not comparable**, not a regression.
Missing coverage and unsupported Sono-GCC parity are also disclosed. Exact
pixels are the default; an optional channel tolerance is capped at 2. Semantic
JSON floating-point values are compared after absolute 1e-9 canonicalization;
integer values and larger numeric differences remain exact. This was added
after an initial real self-comparison reported different last-bit Draw
coordinates and an event time despite identical pixels and audio. Re-running
the same fixture then passed all captured layers. Checkpoint timing is the
median of recorded samples and is classified as changed only when the delta
exceeds both 2 ms and 5%.

## Reference management

Schema version 1 references fingerprint input content and the normalized
effective configuration, and include environment/build metadata, requested
checkpoint schedule, resolutions, mode coverage, and image manifests. UTC
timestamps are metadata, never elapsed duration. References are first captured
to a sibling staging directory and then published atomically. Creating a
reference is explicit; replacing one requires `--replace`, and the CLI/GUI
shows the previous-to-new layer summary. References are never pruned
automatically. Automatically-created run bundles are marked and retained to
the newest 20. Custom output directories remain user-managed.

## Real renderer capture and visual inspection

The repository now has `regression-fixtures/next-rush-larp64x.json` and a
one-second audio variant. They point at the local Larp 64x fixture in
`TestingSuite/Next Sekai Engine/levels/larp 64x`; that ignored fixture must be
present to reproduce the command. The existing default asset-backed UI and
validation watermark profiles are supplied at the API/CLI boundary.

The real CPU capture at
[`artifacts/regression-lab/references/next-rush-larp64x`](artifacts/regression-lab/references/next-rush-larp64x)
contains two checkpoints (24.667 and 24.833 seconds), 16:9 (1280x720) and 4:3
(1024x768) frames, and interpreter/GCC capture attempts: eight PNGs total.
VM and GCC RGB hashes were identical at all four mode/resolution/checkpoint
combinations. Sono-GCC reported **zero native regions** at both checkpoints.
A second short capture at 1.0 seconds also reported zero native regions, so
these observations do not establish interpreter/native parity; the lab marks
that parity unavailable instead of passing it.

I visually opened the 1280x720 and 1024x768 checkpoint images. The renderer
produced valid full-frame chart backgrounds, lane geometry, assets, and a
moving watermark; relative scaling changed with the aspect ratio. The
gameplay HUD is visibly incomplete in this capture: rank/score values are
blank, the life gauge is empty, and combo/judgement elements are absent at
these frames. The watermark text also contains a visibly garbled separator.
This generated reference records current output; it is **not an approval of the
visual design**. The `gameplay` adapter state is null, and frame 297 contains
93 scheduled effects but no immediate audio event. The 24.833 frame contains
no scheduled events. These captures make those gaps inspectable without
inventing judgement results.

Selected lossless RGB PNG SHA-256 values (the hashes are for image files, not
the manifest's raw RGB SHA-1):

| Capture | SHA-256 |
| --- | --- |
| [Interpreter 1280x720, frame 297](artifacts/regression-lab/references/next-rush-larp64x/interpreter-1280x720/checkpoint-00-frame-00000297.png) | `4DDFC04537D45B2997EDB13745A5DA7152B3C9142DCE82D304D84DB8196664E0` |
| [Interpreter 1280x720, frame 298](artifacts/regression-lab/references/next-rush-larp64x/interpreter-1280x720/checkpoint-01-frame-00000298.png) | `492267883851BC2FCDF2B855C9A6609FB0A7F3CCB68B7CC991BC36303E379BC6` |
| [Interpreter 1024x768, frame 297](artifacts/regression-lab/references/next-rush-larp64x/interpreter-1024x768/checkpoint-00-frame-00000297.png) | `10FCB6ED9203EE48DA4B93FE067D3AC7F43F6D27D411334FD90AA19F0A79AC02` |
| [Interpreter 1024x768, frame 298](artifacts/regression-lab/references/next-rush-larp64x/interpreter-1024x768/checkpoint-01-frame-00000298.png) | `5698FD40BED8D752C742E9B24CC10459084A38B8C8668C30DFBB33F2FC200E8B` |
| [Interpreter 1280x720, frame 12](artifacts/regression-lab/references/next-rush-larp64x-gcc-smoke/interpreter-1280x720/checkpoint-00-frame-00000012.png) | `28A7AB87A477BFEB90DD0598C769D4723B3008C28CD7A0B7A05EFDC061671C81` |

The Sono-GCC PNG hashes matched the corresponding interpreter PNG hashes. The
generated `reference.json` has the full per-frame SHA-1 and timing details.
I also completed a real one-second media capture at 16x16 over chart time
2.5 to 3.5 seconds. It submitted 12 frames, produced H.264 video and AAC audio,
recorded 44,100 BGM PCM sample frames, and captured SFX event hash
`065623858bbffa464f9c2da960afbeefd8d4586b` plus pre-AAC SFX PCM hash
`51dc8c89910dda07eb38123b85a6ba755b982c17`. The SFX manifest counted 93
scheduled effects and 3 loop starts/stops; no immediate audio event occurred
at the visual checkpoint. The PNG's SHA-256 is
`92BCA35EE40C933D71E001CCA7B535D27693F36BE2C3565B638CC16D75B74C67`.
The end-to-end `regression run` against this reference passed: pixels, Draw
data after float canonicalization, event layers, watermark/UI state, SFX event
and PCM hashes, BGM timeline, and media metadata matched. The measured
checkpoint-step sample was 3.689 s versus 3.814 s (124.98 ms / about 3.4%),
classified `within_noise` under the 2 ms plus 5% policy. This is a real
audio-pipeline integration capture, not a retained MP4 or a visual reference.
A first 10-second 1280x720 media attempt was canceled after profiling showed
several seconds per stream frame on this CPU; its staged partial capture was
discarded. The documented one-second media recipe keeps the integration check
practical.

## Validation performed

- `cargo check --all-targets` passed on the final source.
- Twelve regression unit tests cover schema/version, missing/corrupt references,
  workload identity, field-level JSON changes, pixel tolerance and spatial
  metrics, diff/heatmap artifacts, timing-noise policy, optional PCM changes,
  1e-9 semantic-float canonicalization, and explicit replacement summaries.
- CLI parsing tests cover repeatable checkpoints/resolutions and profile
  options.
- `cargo check --all-targets` compiles the GUI Regression Lab path; the desktop
  tab was not interactively opened in this validation pass.
- A direct audio mixer assertion verifies normal SFX mixing does not hash PCM
  and the explicit profiling path does.
- The actual Larp 64x renderer capture exercised interpreter and Sono-GCC mode
  selection and both aspect ratios. The renderer correctly disclosed missing
  GCC native coverage.
- A real 12-frame H.264/AAC media export completed at 16x16; its manifest
  contains BGM window metadata, 93 scheduled effects, and a nonempty pre-AAC
  SFX PCM hash.
- An end-to-end rerun against that reference passed (`regression run`,
  `comparable=true`); the first run's tiny exact-float mismatch motivated the
  canonicalization policy above.
- The passing structured result is retained at
  `artifacts/regression-lab/runs/next-rush-larp64x-audio-smoke-tolerant/comparison.json`.
- Correctness tests assert data semantics, not machine-speed thresholds.

## Reproduction

From the repository root, see the complete commands and policies in
[`docs/REGRESSION_LAB.md`](docs/REGRESSION_LAB.md). The primary command is:

```powershell
cargo run --release -- regression update regression-fixtures/next-rush-larp64x.json `
  --reference artifacts/regression-lab/references/next-rush-larp64x `
  --fixture next-rush-larp64x --checkpoint 24.667 --checkpoint 24.833 `
  --resolution 1280x720 --resolution 1024x768 `
  --ui-profile ui-profiles/default.json `
  --watermark-profile watermark-profiles/validation-text.json `
  --compare-vm-gcc --timing-samples 3
```

Run against the created reference using `regression run` with the same config
and profiles. Use `regression inspect <reference>` to see its schema and
coverage. Replace a reference only after reviewing the current PNGs and the
printed change summary.
