# Sono-Renderer Follow-up #32.8 — Presentation Studio convergence

## Outcome

I inspected the current implementation and the #32.5, #32.6, and #32.7
reports before changing anything. The older #32.7 remainder matrix is stale:
the current `composition.rs`, `gui.rs`, and Presentation Studio guide now contain
grouping, nested compositions, proxies, asynchronous thumbnails, curve editing,
adjustment layers, export ranges/presets, drag/drop, and the other systems that
the earlier matrix marked absent.

The current production implementation has no remaining **category A** gap that
blocks the baseline ordinary-video editing and export workflow. This pass did
not add another NLE subsystem. It made two targeted convergence changes:

1. Composition undo/redo snapshot handling is now an isolated, testable helper
   used by the Presentation Studio GUI. A regression exercises media insertion,
   grouped edits, undo/redo, source retention, and save/reopen.
2. The real FFmpeg acceptance fixture now combines source trims, two playback
   rates, a dissolve, and preview/export parity in one composition. This had
   previously been tested as separate capabilities.

I also corrected stale architecture and microphone-integration documentation.
The feature matrix below distinguishes code/test evidence from GUI/device
certification; no GUI interaction is claimed.

## 1. Baseline capability matrix

| Area | Baseline from current code and tests | Baseline evidence / boundary |
|---|---|---|
| Standalone composition | Implemented | `CompositionProject` and its FFmpeg path do not load `RenderConfig`, Watch, an engine, LevelData, or Sono-GCC. Existing real-media fixture uses ordinary files only. |
| Media management | Implemented | Canonical sources, search/relink, bounded asynchronous thumbnails, generated/still media, file drop into the bin, and bin-to-track placement exist. GUI gestures were not manually driven. |
| Timeline editing | Implemented | Move, trim, split, duplicate, copy/paste, slip, insert, overwrite, ripple, groups, linked A/V, track ordering/locks, snapping, zoom, and track height are represented in production model/UI code. |
| Visual composition | Implemented | Ordered effects, keyframes, rectangle/ellipse masks, transitions, text/title presets, generated shapes, adjustment clips, nested compositions, and proxy preview are present. |
| Audio | Implemented | Multitrack mix, linked video audio, imported/recorded WAV, track/clip gain, pan, mute/solo, fades/crossfade action, output meter, and shared preview/export graph exist. Physical device behavior is unverified here. |
| Playback | Implemented | Streaming FFmpeg video/audio preview, play/pause/seek/scrub/frame-step controls, CPAL-sample playback clock, and bounded preview frames exist. Only the FFmpeg pipe path was integration-tested in this environment. |
| Project safety | Implemented | Versioned `.sonocomp`, migrations, unknown-field retention, atomic save, recovery checkpoints, nested-cycle checks, relinking, and source-preserving undo are present. |
| Export | Implemented | H.264/AAC software export, presets, In/Out range, cancellation, staged publication, and FFprobe validation are present. Landscape, portrait, range, and nested exports are in the real fixture. |
| Packaged app | Partial evidence | An existing portable package passed the non-GUI CLI smoke script. It was not freshly packaged from the #32.8 working tree and its GUI was not opened. |

## 2. Remaining category-A gaps discovered

**None identified in the current production code for a coherent baseline NLE.**
The earlier #32.7 rows for groups, nesting, proxies, thumbnails, keyframe graph,
adjustment layers, audio crossfades, named software-export quality, range export,
and drag/drop are implemented in the current repository. This conclusion is
based on present code plus model/real-FFmpeg evidence, not on the obsolete
#32.7 matrix.

This means the production feature set can express the requested ordinary
workflow. It does **not** mean a human completed the full desktop acceptance
sequence in this environment.

## 3. Implementations completed in this pass

- `CompositionEditHistory` owns the bounded 40-snapshot undo/redo stacks and
  the media-source retention rule. The GUI delegates to it instead of owning
  the stack transitions inline.
- Added `presentation_insert_group_undo_redo_roundtrips_one_authoritative_project`.
  It confirms that undoing an insertion removes its timeline clip while keeping
  the media source, that redo restores the clip, that grouping is undoable, and
  that the resulting authoritative project survives save/reopen.
- Extended the ignored real-media fixture with trimmed 0.5x/0.75x clips under a
  cross-dissolve and a decoded-MP4-vs-preview pixel comparison.
- Updated `docs/ARCHITECTURE.md`, which incorrectly described Presentation
  Studio as a time-sorted list with a still-frame preview. Updated the audio
  input guide to match current gain automation, track gain, and crossfade UI.

No Watch, Sono-GCC, gameplay, output-encoding, or Sonolus semantics were changed.

## 4. Integration bugs found/fixed

The real-media fixture previously tested source-time mapping and transitions
independently. That left their composition unproven. The new check exposed no
renderer defect: the combined preview and exported frame agreed within the
lossy H.264 tolerance. The GUI undo stack had no direct test for a grouped edit
or for keeping imported media after undo. It now has one against the same
`CompositionProject` state used by the editor.

Other tested interactions include effect stack plus mask, nested child edits
propagating into the parent, proxy preview with original-only final export,
linked A/V edits, and track-lock rejection without partial ripple/drop edits.

## 5. Timeline usability results

**Build-checked / code-inspected:** timeline zoom, scrolling, adjustable track
height, snapping toggle, playhead, selection/group controls, visible lock state,
clip trim/move handles, media-bin drag handles, and inspector state are present.
Keyboard shortcuts are suppressed when a text editor owns input.

**Not interactively exercised:** selection accuracy, handle size, drag ambiguity,
scroll/zoom feel, monitor layout, shortcut focus, recovery dialogs, and the
actual media-bin-to-track gesture. This session has no desktop-control tool.
Therefore the GUI usability step is not certified green.

## 6. Playback results

- **Integration-tested:** ordinary-media FFmpeg preview emits a real frame and
  non-silent audio PCM; the continuous decode-pipe lifetime test passes.
- **Code-inspected:** the timeline clock follows submitted CPAL output sample
  frames. Preview decoding can hold/drop frames rather than slowing that clock.
- **Not playback-tested on hardware:** play/pause/resume, repeated seek,
  scrubbing, A/V latency, long-run drift, or sustained-load frame drops on a
  physical audio output. No CPU/GPU/RAM trend was recorded.

## 7. Audio results

The production graph mixes ordinary WAV, routed video audio, and recorded WAV
clips. The real fixture decodes 440 Hz output and verifies that 0.5x audio-track
gain approximately halves PCM RMS. Existing model tests cover mute/solo/pan and
crossfade construction. No physical output or microphone was used, so the
hardware meter, voiceover capture, clipping, device removal, and A/V sync remain
unverified.

## 8. Project torture-test results

The ignored real-media fixture creates a standalone project from three generated
MP4s, two WAVs, a video-with-audio file, PNG/JPEG stills, and generated sources.
It uses multiple visual/audio tracks, linked A/V, effects, masks, transitions,
text, source-time mapping, and generated graphics; it saves/reopens, streams
preview, and exports. Model tests cover grouping, locks, ripple, keyframes,
recovery, and serialization.

This is production-compositor/media coverage, not a mouse-operated long-form
editing session. A 10+ minute editor session with timeline zoom/scroll, repeated
seek, sustained playback, and memory monitoring was not run.

## 9. Vertical project results

The real fixture renders and FFprobe-validates 1080x1920 H.264/AAC output from
landscape media. I inspected
[`standalone-vertical-1080x1920.png`](../../../artifacts/nle-validation/standalone-vertical-1080x1920.png).
It contains the expected reframed landscape layer over a blurred/color-treated
fill. The frame is intentionally a synthetic diagnostic composition, not a
polished creator edit. The project was not assembled through the desktop UI.

## 10. Longer-project results

**Not tested.** The fixture has a short timeline and multiple overlapping
features, but it is not a large/long edit. The absence of a UI automation path
prevents the requested timeline zoom/scroll/playback/seek sequence; no long
project performance claim is made.

## 11. Undo/redo results

**Unit-tested after the refactor:** insert → undo → source retained → redo →
group → undo → redo → save/reopen. Existing editing operations continue to
snapshot the same `CompositionProject`; the new test protects source ownership
and authoritative model state, not just GUI appearance.

## 12. Failure/recovery results

The full normal and serial suites cover malformed/future schemas, missing
references, relative paths, nested cycles, lock failures, invalid edits,
recovery snapshots, media relinking checks, and staged export cleanup. No new
failure-injection run covered disk-full, missing FFmpeg, malformed codec,
corrupt project recovery UI, or device loss. Those cases remain unverified.

## 13. Export evidence

**Real-media tested:** the ignored fixture exports landscape, vertical, In/Out
range, retimed, nested, and proxy-related outputs through production FFmpeg
paths and validates selected outputs with FFprobe. H.264/AAC is software
encoding. Hardware encoder selection is not implemented; no hardware encoder
was available or tested.

## 14. Preview/export parity

The test compares a frame-evaluated brightness preview with a decoded MP4 frame,
and now compares the frame from a transition combining source trims and two
playback rates with the matching export frame. Mask/effect order, transitions,
nested output, and proxy original-only export also have production pixel/frame
assertions. These are semantic/pixel checks on representative fixtures; they do
not establish parity for every filter, font, codec, or real-world source.

The new inspected image is
[`transition-trimmed-retimed-preview.png`](../../../artifacts/nle-validation/transition-trimmed-retimed-preview.png),
640x360 at timeline time 1.5 seconds, SHA-256
`FFC18F33CD78A266D5CAD98A4D970ED9DE24F3EF971E6E01048751D137D6D396`.
The source test-pattern clock reads approximately 0.63 seconds, as expected
from its 0.25-second source in-point plus 0.5 seconds at 0.75x. The frame shows
the intended dissolve and no canvas-edge clipping.

## 15. Packaged-app results

**Portable CLI smoke-tested:** the existing Windows portable ZIP passed
`smoke-test-windows-package.ps1 -PortableOnly`, including extraction into a
path with spaces/Unicode, launch from a foreign working directory with a
restricted `PATH`, and `--version`, `--help`, `jobs list`, and media tool info.
This was a non-GUI smoke against the already-present package. I did not run
`package-windows.ps1` because it deletes/replaces the existing
`target/distribution` and `target/release-stage` directories; those held prior
package artifacts. The current working tree release binary was release-built,
but a fresh packaged GUI was not tested.

## 16. Performance results

No NLE-specific preview FPS, dropped-frame, CPU/GPU, memory-growth, or seek
latency measurements were collected. The existing authoritative Performance
Laboratory instruments Sonolus render jobs and does not describe interactive
composition playback. The NLE uses bounded worker/thumbnail/proxy paths, but
this pass makes no claim that a long session has bounded process memory under
all media workloads. No optimization was made without evidence.

## 17. Windows abort observations

Both `cargo test --locked` and a subsequent
`cargo test --locked -- --test-threads=1` completed with exit code 0 and no
`STATUS_HEAP_CORRUPTION`, access violation, or harness abort in those runs.
The serial run used the pre-extraction test binary; after the small undo-helper
refactor, the new targeted test, all-target check, Clippy, and release build
passed. A clean run does not resolve the historical intermittent Windows
abort.

## 18. Grandpa regression results

The full renderer library, CLI, `tests/m0.rs`, and Watch scheduler suites passed
in the full test runs. The exact results were: library 364 passed / 6 ignored,
main 27 passed, `m0` 101 passed, scheduler 9 passed. The serial full run also
completed with exit 0. No real-engine visual/audio artifact set was rerun in
this NLE pass; no claim is made that NLE tests replace VM/GCC differential or
historical engine validation.

## 19. Final capability matrix

| Capability cluster | Current status | Evidence |
|---|---|---|
| Standalone ordinary-media project | **Implemented + integration-tested** | No Sonolus dependency in composition path; real media fixture. |
| Media bin, thumbnails, relink, drag/drop | **Implemented + model/build-checked** | Current code, thumbnail tests, track-drop compatibility/atomicity test; GUI gesture not manually driven. |
| Core timeline edits, groups, linked A/V, locks | **Implemented + unit-tested** | Production model tests; group insert/undo/redo save-reopen test. |
| Effects, masks, transitions, keyframes, titles, shapes, adjustment layers | **Implemented + real-render tested** | Production FFmpeg frame assertions and inspected artifacts. |
| Constant speed/reverse/freeze | **Implemented + real-render tested** | Preview/export path fixtures; audio retiming changes pitch. |
| Nested compositions and proxies | **Implemented + real-render/export tested** | Child edit propagation, cycle validation, proxy preview/original export parity. |
| Continuous preview pipeline | **Implemented + pipe integration-tested** | FFmpeg video/PCM stream test; physical CPAL session not tested. |
| Timeline audio and export mix | **Implemented + graph/PCM tested** | Gain RMS and mixer tests; physical devices/drift not tested. |
| Atomic project save/recovery/export | **Implemented + model/integration-tested** | Save/reopen, migration/recovery tests, staged/FFprobe export. |
| Desktop workflow | **Not interactively verified** | GUI compiles; no desktop-control facility in this session. |
| Long-form playback/performance | **Not verified** | No long project, soak, frame-drop, or memory measurement. |
| Packaged desktop NLE workflow | **Not verified** | Existing portable CLI smoke only; no fresh package GUI session. |

## 20. Remaining ordinary-NLE gaps

No category-A gap was identified in the baseline editing feature set. Human
operation is still unverified, so this is not a claim that the application has
passed the user's interactive acceptance criterion. The most important next
evidence is a human-operated edit/export session, plus a long-form playback
session on a physical output device.

## 21. Advanced/future scope

These do not block the current baseline NLE claim and were not added here:

- Variable-speed ramps and pitch-preserving time stretch.
- Bezier/freeform masks, chroma-key spill suppression, and advanced audio FX
  such as EQ/compression.
- Cubic-Bezier keyframe handles, hardware encoding, and a persistent
  multi-export queue/history for compositions.
- Live Watch-backed gameplay as a composition source; pre-rendered gameplay
  remains ordinary video media.
- Cloud collaboration, plugin marketplace, general-purpose DAW/image editor,
  and other explicitly out-of-product systems.

## 22. Exact blockers

- **Interactive GUI certification:** no UI automation/desktop interaction
  capability was exposed to this session. The GUI was build-checked only.
- **Physical audio certification:** no physical capture/output latency,
  microphone drift, device hotplug, or live meter test was performed.
- **Long-session certification:** no 10+ minute/large-clip playback, export,
  memory, CPU/GPU, or seek-latency run was performed.
- **Fresh package GUI certification:** existing package outputs were preserved;
  the packaging script would clear those directories. The existing portable
  package received only its non-GUI CLI smoke.

These are evidence/environment limits, not claims that the corresponding
production systems are absent.

## 23. Evidence classification and commands

| Check | Result |
|---|---|
| Baseline `cargo fmt --all -- --check` | Passed; Cargo emitted a non-fatal path canonicalization warning. |
| Baseline `cargo check --locked --all-targets` | Passed; build warned `rc.exe` was unavailable for executable version metadata. |
| Full normal `cargo test --locked` | Passed: library 364/6 ignored, main 27, m0 101, scheduler 9; no process abort. |
| Full serial `cargo test --locked -- --test-threads=1` | Passed, exit 0, no process abort; run preceded the undo-helper extraction. |
| Targeted undo/history regression after extraction | Passed. |
| `cargo check --locked --all-targets` after extraction | Passed. |
| Ignored production FFmpeg project fixture after transition+retime addition | Passed; 68.11 s. |
| Ignored continuous video-pipe fixture | Passed; 0.49 s. |
| `cargo clippy --locked --all-targets` | Completed successfully with existing repository-wide warnings. |
| `cargo build --locked --release` | Passed. |
| Existing portable-package CLI smoke | Passed; GUI not exercised. |
| Visual inspection | Inspected landscape, vertical, eased transition, proxy preview, nested child, and trimmed/retimed transition frames. Synthetic frames showed expected composition states; no unexpected canvas-edge clipping or missing media was visible. |

Focused reproduction commands:

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --lib gui::tests::presentation_insert_group_undo_redo_roundtrips_one_authoritative_project
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture
```

The real-media fixture requires the already-installed application-managed
FFmpeg/FFprobe pair and writes PNGs under
[`artifacts/nle-validation`](../../../artifacts/nle-validation/README.md).

## Architecture freeze recommendation

Freeze `CompositionProject` as the standalone NLE document; `MediaSource` as
the canonical media-bin identity; `CompositionClip`/tracks/transitions/effect
instances as the editing model; the shared FFmpeg composition graph as preview
and export semantics; and `CompositionEditHistory` as the bounded editor
snapshot history. Keep Sonolus `RenderConfig`, Watch execution, and render
history separate from generic composition jobs. Add future NLE work through
these existing boundaries and require preview/export parity evidence for new
timeline semantics.
