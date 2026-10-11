# Follow-up #30: standalone Presentation Studio

## Implemented

Presentation Studio now has a standalone composition path in the existing
desktop application. A new composition defaults to 1080 x 1920 at 30 FPS for
30 seconds. The composition schema and execution backend live in
[`src/composition.rs`](src/composition.rs); the workspace and asynchronous
preview/export controls live in [`src/gui.rs`](src/gui.rs).

The format is `sono-renderer-composition` schema version 1 (`.sonocomp`). It
stores canvas settings, media source references, clip timing, visual
transforms/crop/opacity/blur, text styling, audio routing/gain/fades, z-order,
and eased horizontal-position keyframes. Source bytes are not embedded.
Relative paths are stored where they can represent the source, and unsupported
future schema versions fail explicitly. Unknown top-level, source, and clip
fields are retained through flattened extension maps.

The backend imports ordinary local media by probing actual streams, splits a
clip while preserving source-time continuity, duplicates visual instances
muted, builds one FFmpeg filter graph for preview/export, mixes routed audio,
renders text, and validates the output MP4. It does not use Sonolus engines,
LevelData, WatchVm, Sono-GCC, gameplay events, or Sonolus resource decoding.
Composition work runs in an owned cancellable background task and is polled by
the GUI. Output collision checks prevent overwrite.

## Acceptance actions represented in the UI

- New project and selectable landscape, 9:16, square, and custom dimensions.
- Import video/audio files and add them as clips.
- Trim by source in-point/duration, split at playhead, duplicate, reposition,
  crop, scale, change z-order, and apply Gaussian blur.
- Add styled text and horizontal position keyframes with linear/ease-in/
  ease-out/ease-in-out interpolation.
- Mix independent audio clips, route video audio explicitly, set gain and
  fades.
- Seek the playhead and render a still preview frame; save, reopen, and export
  H.264 MP4 with AAC when audio is routed.

## Validation

- **Build-checked:** `cargo fmt --all` and `cargo check --locked` succeeded.
- **Tested:** 9 focused composition tests passed. They cover vertical/square/
  landscape dimensions, fit-to-canvas defaults, split/duplicate behavior,
  keyframe/easing filter generation, source fan-out and audio graph creation,
  seek-frame selection, relative-path save/reopen and overwrite, future-schema
  rejection, and absence of engine/level/Watch fields.
- **Tested:** the full `cargo test --locked` suite passed: 440 passed and 5
  ignored across the library, CLI, and integration test targets.
- **Not run:** the real FFmpeg integration test is opt-in because this machine
  has no app-managed FFmpeg/FFprobe installation. It is available as
  `cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture`.
  That test generates ordinary MP4/WAV inputs and then exercises the real
  import, save/reopen, preview, and MP4 export paths without downloading tools.
- **Not tested:** interactive GUI behavior and a completed MP4 encode in this
  environment. GUI compilation passed, but no claim is made that the editor
  was visually/interaction tested.

## Known limits

The current timeline is a time-sorted clip list with editable start/end values,
not draggable clip blocks or audio waveforms. Preview is a still-frame seek
monitor and does not play video/audio continuously. The editing model currently
animates horizontal position only; scaling, opacity, and audio automation are
not keyframed. A previously rendered Sonolus MP4 can be imported as local
video, but live Watch-backed gameplay is not yet a composition source type.
Media probing runs when import is selected, and the first use may provision
the pinned FFmpeg tools if absent. These are explicit limits of this first
general composition path.

Further usage and schema/path semantics are in
[`docs/PRESENTATION_STUDIO.md`](docs/PRESENTATION_STUDIO.md).
