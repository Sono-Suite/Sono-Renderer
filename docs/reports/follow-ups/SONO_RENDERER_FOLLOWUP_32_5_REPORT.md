# Sono-Renderer Follow-up #32.5 — Presentation Studio NLE Maturation

## Summary

Presentation Studio now has a real multitrack composition model, a timeline editing surface, reusable animated properties, continuous video/audio preview, bounded recovery checkpoints, and a standalone real-media acceptance fixture. The composition path remains independent of Sonolus: it uses ordinary media sources and the shared FFmpeg composition graph. No Watch VM or Sono-GCC path is entered by creating or editing a composition.

**Acceptance status: implemented and substantially integration-tested; the full interactive acceptance workflow is not claimed as passed.** The production composition backend was exercised with real FFmpeg/FFprobe, synthetic video/audio files, project save/reopen, preview, and two MP4 exports. The desktop GUI was build-checked but not interactively driven in this environment. Physical CPAL playback and microphone A/V synchronization were not tested.

## 1. Architecture found and changes made

Before this pass, `CompositionProject` already owned ordinary media sources, clips, project persistence, composition filtering, still-frame rendering, and FFmpeg export. Presentation Studio was still mostly a clip-list/still-preview workflow. Track identity and ordering were incomplete, keyframes had narrow special cases, and the editor did not have a continuous playback lifecycle. The composition code itself did not require Sonolus, which remains the correct boundary.

This pass extended the existing composition and GUI systems rather than creating a second editor. `CompositionProject` is still the persistent/editable composition; `MediaSource` remains the canonical media identity; `CompositionClip` is a timeline instance; tracks arrange instances; the FFmpeg graph remains the single preview/export compositor; and CPAL remains the preview audio output. `VideoPipe` now owns the temporary filter/script workspace for the lifetime of the FFmpeg decoder. This fixes an observed lifetime bug where text/animated-filter inputs could disappear while a playback decoder still needed them.

The most substantial changes are in [composition.rs](../../../src/composition.rs), [gui.rs](../../../src/gui.rs), and [timeline_playback.rs](../../../src/timeline_playback.rs). User and developer behavior is documented in [PRESENTATION_STUDIO.md](../../PRESENTATION_STUDIO.md) and [PRESENTATION_STUDIO_AUDIO_INPUT.md](../../PRESENTATION_STUDIO_AUDIO_INPUT.md).

## 2–10. Tracks, clips, timeline, edits, properties, and keyframes

- **Implemented:** Stable track IDs, names, visual/audio kinds, ordering, enabled state, and audio mute/solo. Reorder is constrained to the matching visual or audio stack. Clips reference a canonical media source and a compatible track. Removing a nonempty track is rejected.
- **Implemented:** Timeline UI with time ruler, track rows, clip blocks, playhead, zoom/scroll, clip selection, waveform summaries, and visible keyframe markers. Clip movement and edge trims snap to frame/playhead/nearby clip edges. Track changes use drag/drop between compatible rows. Editing is non-ripple; gaps remain explicit.
- **Implemented:** Move, trim, split, duplicate, copy/paste, delete, track creation/reorder/removal, and ordinary audio/video clip arrangement. Duplicate clips reference the same media source; media bytes are not copied.
- **Implemented:** A common scalar animated-property map covers X/Y, scale X/Y, rotation, opacity, crop edges, blur, audio gain/pan, and text font size/outline/shadow offsets. Values validate as finite and property-bounded; duplicate-time keyframes are rejected. Boundaries hold the first/last value. Easing supports linear, hold, ease-in, ease-out, and ease-in-out. No cubic-Bezier editor or keyframe clipboard is claimed.
- **Implemented:** Inspector controls expose animated state, set/add/edit/remove keys, easing, current key navigation, crop/blur, and text style/font/alignment. A full curve editor and multi-selection are not implemented.
- **Implemented:** FFmpeg composes simultaneous visual tracks and clip layers with transforms, opacity, crop, blur, and text. Animated crop/blur use a bounded generated command schedule; long schedules are rejected rather than growing without bound. Text supports selected local TTF/OTF/TTC fonts and system fallback.
- **Unit-tested:** Track assignment/order, split continuity, duplicate behavior, keyframe boundaries/easing, position interpolation, v1 migration, path round-trip/unknown fields, filter graph routing, and preview frame selection.

## 11–17. Playback, audio, waveforms, A/V, undo

- **Implemented:** Play/pause/stop, seek, scrub, frame stepping, time display, and playback from an arbitrary playhead. The audio device's submitted sample-frame count is the timeline playback clock; GUI repaint frequency is not authoritative. Preview video is downscaled to a maximum 960-pixel edge and consumes frames against that clock, dropping/holding frames if decode falls behind. It does not slow the timeline to match render completion.
- **Implemented:** Preview uses the composition's FFmpeg graph for both video and audio. Audio sources from imported audio, routed video audio, and recorded WAV clips use the same mixer/filter path as export. Overlap, mute/solo, gain, fade, and pan are supported. Existing bounded source peak summaries are displayed on audio clips and remain shared by imported and recorded audio.
- **Integration-tested:** The real-media test starts the continuous video pipe, reads a valid frame, renders the audio pipe, and verifies non-silent mixed PCM. A dedicated ignored real-FFmpeg pipe test verifies filter files live through decode completion.
- **Implemented:** Undo/redo keeps up to 40 bounded composition snapshots for edit operations; media bytes remain outside those snapshots. Undoing a recorded clip insertion keeps the audio source available. Keyboard bindings include Space, arrows, Delete, S, Ctrl+C/V/D/Z, Ctrl+Y, and Ctrl+Shift+Z, suppressed while text fields own input.
- **Not verified:** Interactive play/pause/seek behavior on a physical audio output device; physical A/V latency; long-duration clock drift; and whether preview stays perceptually synchronized under sustained CPU/GPU load. The backend test checks production graph output, not a real CPAL speaker session. No unexplained latency offset was added.

## 18–24. Persistence, recovery, relink, and export

- **Implemented:** `.sonocomp` schema v2 persists tracks, clips, trims, transforms, effects, text, audio properties, and keyframes. Schema v1 migrates on load. Unknown fields survive round-trip; unsupported future versions are rejected without rewriting the source. Project saves use atomic replacement and keep media external.
- **Implemented:** A versioned, atomic recovery checkpoint is separate from the explicit project save. It is written after 20 seconds idle while Presentation Studio is active, keeps at most 20 newest snapshots, and exposes explicit recover/remove actions. Recover restores a working copy; it does not silently overwrite the saved project.
- **Implemented:** Missing source paths remain visible. Relinking probes and validates the replacement before updating the canonical source, repairing all clips that reference it. There is no same-name automatic substitution.
- **Implemented:** Preview and export share the same composition graph. Export stages output, validates the finished MP4 with FFprobe, then publishes it. Existing output is not silently overwritten. The renderer supports the configured canvas/FPS, H.264/AAC output when routed audio exists, cancellation, and diagnostic errors.
- **Real-media/export-tested:** The opt-in integration fixture creates ordinary MP4/WAV files, saves and reopens a `.sonocomp`, renders landscape and vertical previews, and exports both aspect ratios. The production export path's FFprobe validation checks container duration, dimensions, video stream, and routed audio stream.
- **Not tested:** Cancellation, unwritable destination, FFmpeg-missing, malformed/unsupported codecs, and corrupt recovery file scenarios were not exhaustively exercised in this pass.

## 25–29. Acceptance fixture, microphone, Sonolus boundary, resource behavior

The current real-media fixture begins with no Sonolus data. It generates three ordinary three-second videos and two WAV sine sources, creates a 1920×1080 composition, trims the base video at source in/out, uses multiple visual/audio tracks, splits and duplicates clips, overlaps layers and audio, adjusts crop/blur/transforms, animates X/Y/scale/rotation/opacity/crop/blur, adds text, mixes gain/pan/fades, saves and reopens, exercises playback graph pipes, and exports landscape plus a vertical variant. A preview-size check also covers 802×450. There is no Watch VM, EngineData, LevelData, gameplay event, or Sonolus resource in this test.

- **Implemented / real-media tested:** Ordinary standalone composition workflow through production composition rendering and export.
- **Implemented / not physically tested:** Follow-up #32 microphone capture produces an ordinary WAV media source/clip and enters this same audio timeline/mixer/waveform/undo path. Physical recording, device disconnect, permissions, long recording, and packaged-app capture remain unverified. Details are in the linked microphone document.
- **Boundary preserved:** Live Watch-backed gameplay is not currently a composition source. A pre-rendered gameplay MP4 is ordinary video media. Live gameplay integration remains intentionally deferred.
- **Resource safety:** The video reader and capture queues are bounded; preview frames are reduced in size; autosave/recovery is bounded; filter command generation has a limit; undo history has a fixed cap. No long-session memory/seek stress benchmark was run, so these bounds are not a claim of proven real-time performance under every workload.

## 30–35. Regression, GUI, limitations, deferrals

Validation performed:

- **Build-checked:** `cargo check --locked` passed. The environment warned that `rc.exe` is unavailable, so executable version metadata was not embedded.
- **Lint-checked:** `cargo clippy --locked --all-targets` completed without errors. It reported existing and current style warnings, including `runtime.rs:1683` and argument-count/style warnings; no broad lint cleanup was part of this NLE task.
- **Formatting/patch checks:** `cargo fmt --check` and `git diff --check` passed. Git emitted line-ending conversion notices for repository files.
- **Unit-tested after final fixture change:** `cargo test --locked --lib composition::tests` passed 15, with the real-media test ignored by default; `cargo test --locked --lib timeline_playback::tests` passed 2, with the FFmpeg pipe test ignored by default.
- **Real-media tested after final fixture change:** `cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture` passed. The dedicated pipe test also passed earlier in this pass.
- **Regression suite:** The complete library suite passed serially before the final fixture-only adjustment: 314 passed, 0 failed, 6 ignored. An earlier parallel run ended with Windows `STATUS_HEAP_CORRUPTION` without a named failing test; serial execution passed. Following the fixture adjustment, its real integration test and the focused composition/playback unit suites passed again. The complete library suite was not rerun after this test-fixture-only adjustment.
- **GUI:** Windows `cargo check`/all-target lint compiled the current GUI. No interactive mouse/keyboard session was available, so visual editor operations, device playback, recovery dialog, and voiceover workflow have not been manually exercised.
- **Image inspection:** I opened both generated PNGs. The landscape frame shows the test-pattern source, red/blue overlays, rotation/opacity treatment, and caption; the vertical frame shows the fitted 16:9 source with expected black bands plus overlays/caption. Text and overlays are within the frame. These are diagnostic synthetic assets, not finished visual design.

Not claimed as verified: full mouse-driven standalone acceptance; multi-selection; physical CPAL playback; real microphone recording; capture/playback sync and drift; packaged application behavior; live Watch source; every output codec/resolution; stress-scale undo/memory behavior; or the listed failure-injection matrix. Curve editing, keyframe clipboard, multi-select, high-speed jog/shuttle, and live Watch source are intentionally deferred.

## Reproduction and visual artifacts

From the repository root on a machine with app-managed FFmpeg/FFprobe available:

```powershell
cargo test --locked --lib composition::tests -- --nocapture
cargo test --locked --lib timeline_playback::tests
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture
```

The real fixture writes [landscape PNG](../../../artifacts/nle-validation/standalone-landscape-1920x1080.png) and [vertical PNG](../../../artifacts/nle-validation/standalone-vertical-1080x1920.png). The fixture description, frame time, reproduction command, and SHA-256 hashes are in [the artifact README](../../../artifacts/nle-validation/README.md).
