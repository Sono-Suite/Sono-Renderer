# Sono-Renderer Follow-up #24 Report

## Delivery status

**The requested nonlinear editor is not complete.** Repository inspection
found that Follow-up #23 delivered a partial Presentation Studio slice, not
the composition and media-timeline foundation assumed by Follow-up #24. This
pass adds one real GUI workflow on top of existing production code: a
renderer-backed, seekable still-frame Program Preview. It does not claim or
implement an NLE.

## What the current repository actually contains

- `RendererUiConfig` in `src/render_ui.rs` has numeric and categorical profile
  keyframe tracks. The renderer evaluates these while composing gameplay UI.
- `RenderConfig` in `src/render.rs` describes one Watch start/duration range,
  optional music, and one optional full-frame MV background.
- `RenderConfig::render_frame` runs the shared Watch and compositor path for
  a requested global time. Existing MP4 export uses the same configuration
  boundary and the regular job/queue/export pipeline.
- `RenderProject` in `src/project.rs` persists renderer input references and
  render defaults. It does not persist an editing sequence or media bins.
- The GUI has project/input, render, gameplay UI, queue, history, diagnostics,
  and other product tabs. It had no Program Preview, general timeline, source
  viewer, media bin, undo stack, or editing command system.
- `src/export_mv.rs` decodes a single sequential MV background for export. It
  is not a general multi-clip decoder/compositor and does not provide arbitrary
  media seeking or video/audio tracks.

These findings agree with the explicit deferred-work section of
`SONO_RENDERER_FOLLOWUP_23_REPORT.md`.

## Implemented in this pass

The GUI now has a **Program preview** tab. It provides a time slider, Start
and End navigation, frame-back/frame-forward buttons, and left/right arrow
frame stepping. A requested frame runs on a background worker through the
existing `RenderConfig::render_frame` implementation and is displayed as an
egui texture. It does not introduce a second Watch evaluator or rendering
path.

Each request captures the current `RenderConfig`. Generation checks and
configuration equality discard stale results. If a newer request arrives
while a frame is rendering, only the latest queued request is started next.
The monitor downsizes preview output to a maximum dimension of 1920 while
preserving aspect ratio; it leaves export settings untouched. MP4 export is
disabled while a preview worker is active to avoid overlapping renderer work.

User steps and limitations are documented in
[`docs/PROGRAM_PREVIEW.md`](docs/PROGRAM_PREVIEW.md).

## Acceptance-area status

| Area | Status in current repository / this pass |
|---|---|
| Existing project/input workflow | Existing and reused |
| Program monitor, time seek, frame stepping | Implemented as still-frame preview |
| Production Watch/compositor path | Reused through `RenderConfig::render_frame` |
| Stale asynchronous preview suppression | Implemented |
| Responsive preview resolution cap | Implemented; aspect-ratio unit-tested |
| Preview sequential playback / dropped-frame policy | Not implemented |
| Source monitor and source In/Out | Not implemented |
| Media bin, bins/folders, metadata, relink | Not implemented as editor features |
| Multi-track sequence/composition and clip model | Not present |
| Move/trim/split/ripple/roll/slip/slide/insert/overwrite | Not implemented |
| Undo/redo, centralized editing commands/shortcuts | Not implemented; arrow keys only navigate preview frames |
| Audio timeline, waveform, meters, mixing controls | Not implemented |
| Effects/transitions/keyframe curve editor | Not implemented as editing workflows; profile UI tracks remain data/profile authored |
| Proxy and render-in-place workflow | Not implemented |
| Editor sequence save/reopen/autosave/recovery | Not implemented; existing RenderProject remains input-oriented |
| Export, queue, history, manifest, telemetry | Existing renderer workflow remains intact; there is no composed-sequence export |
| Watermark and Watch semantics | Existing paths unchanged |
| Headless/CLI direct render | Unchanged |

## Architecture and behavior notes

No new project, job queue, export pipeline, cache, history, timing system, or
composition schema was added. `ProgramPreviewFrame` and
`ProgramPreviewJob` are ephemeral GUI transport/state only. A preview does not
change `RenderConfig`, project data, Watch state, or queued jobs. The
configuration snapshot used by a preview is checked before its result can
replace the displayed image.

Preview is not real-time: the existing API creates a fresh frame session for
each request and advances Watch from its normal initial state to the requested
global frame. The preview worker is not cancellable; closing waits for it.
Whole Chart's final end is inferred by export and is not available to the
preview slider, which displays the configured start/duration window.

## Validation

- `cargo fmt --all`: completed.
- `cargo check --offline`: passed. Existing warnings remain for deprecated
  `Atomic::fetch_update` and unused `ProgressTracker::pulse`.
- `cargo test --offline --lib program_preview_caps_resolution_without_changing_aspect_ratio`:
  passed (1 test).
- `cargo test --offline --lib next_rush_shared_preview_initializes_baumkuchen_at_cli_options`:
  passed (real-engine shared-preview fixture, 6.40 seconds).
- Full test suite, all-target Clippy, release build, interactive GUI exercise,
  and preview screenshot/visual inspection were not run for this pass.
- No real editor edit/export sequence exists to test end-to-end. The real
  engine test validates the existing renderer preview API called by this GUI
  workflow, not timeline editing.

## Remaining work / conclusion

The requested workflow from importing media through cutting, arranging,
compositing, audio editing, undo/redo, saving, and exporting a composed
sequence is still outstanding. That work cannot honestly be exposed as a
set of controls against the current one-segment `RenderConfig`. The missing
composition/sequence boundary, its project snapshot semantics, and its shared
preview/export evaluator must be implemented and validated before presenting
Sono-Renderer as a nonlinear editor. Follow-up #24's absolute acceptance
criterion is therefore **not met** by this pass.
