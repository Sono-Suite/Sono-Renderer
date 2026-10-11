# Current limitations (reviewed for Follow-up #48)

This list describes the checked-in product after the Follow-up #48
convergence review. A feature being present in code does not mean every
device, workflow, or packaged build has been validated. Supporting evidence
is linked from [the feature audit](../FEATURE_EXISTENCE_AUDIT.md), [the #39
certification](../RELEASE_CANDIDATE_CERTIFICATION.md), [the #40.5
stabilization report](../SONO_RENDERER_FOLLOWUP_40_5_STABILIZATION.md), and
[the #48 convergence report](../SONO_RENDERER_FOLLOWUP_48_CONVERGENCE.md).

The Watch-operation status below was updated by the pragmatic compatibility
closure on 2026-10-08; older #48 reports preserve the findings from their own
checkpoints.

## Renderer and charts

- The generic Watch implementation is incomplete for some Sonolus v1.1.4
  content. Stack operations, `Random`/`RandomInteger`, and mutable `StreamSet`
  execute with documented compatibility approximations; their exact
  official-client pointer/frame ABI, random sequence/state lifecycle, and
  mutable-stream lifecycle are unverified. Set Watch Compatibility to Strict
  to reject these operations, or use the default Permissive policy to run
  them with one warning per family per render job. Replay stream data can be
  attached through the Watch/offline frame APIs. Curved Draw operations use a
  tessellated CPU/GPU path, but have not been pixel-compared with a real
  Sonolus client in both render modes. Check the compatibility report before
  relying on an unfamiliar engine.
- Chart Studio has a generic structural/raw editor for supported canonical
  LevelData and can preserve unknown JSON fields through export/reimport. It
  is not an engine-specific spatial chart editor and cannot infer gameplay
  semantics or entity relationships from compiled Watch behavior. Rich
  authoring requires an optional content-matched declarative schema; no
  engine-authored rich schema is bundled. Runtime compatibility remains a
  separate question. The renderer can still be used directly with engine,
  level, resources, and music without opening Chart Studio.
- Sono-GCC is optional and compiles only eligible regions on supported
  Windows toolchains. It requires `rustc` for native compilation and can fall
  back to the Watch VM. The VM remains the semantic reference. GCC is not
  guaranteed to make a complete render faster.

## Presentation Studio

- A composition can use rendered gameplay as ordinary video, but it cannot
  embed a live Watch session or bind authoritative Watch state directly.
- Masks are limited to the implemented rectangle and ellipse primitives;
  freeform paths and mask combinations are not available. Blend modes beyond
  normal alpha, several advanced transitions, rich text spans, and some of the
  advanced style controls requested in Follow-up #37 are not implemented.
- Template responsiveness uses normalized/scaled layout rules and a
  full-canvas cover rule; it is not a general constraint or breakpoint system.
  Review portrait layouts manually, especially inset and multi-view designs.
- The NLE compositor uses FFmpeg filters and project media. Preview and export
  integration tests pass, but a human-operated desktop editing/export session
  has not been certified in this review.

## Audio, tempo, and subtitles

- Microphone recording is implemented, but real-device enumeration, operating
  system permissions, hot-plug, long-duration drift, and packaged-device
  capture were not verified in this review. Speaker monitoring is not
  implemented. Captures stay local and begin only after an explicit action.
- Automatic subtitles use an optional local Whisper-compatible runtime and
  model. No model is bundled by default; real-speech accuracy, multilingual
  behavior, and packaged inference were not verified here. No cloud upload is
  performed by this local transcription path.
- Tempo analysis is available, but it has not been validated against a labeled
  real-song corpus and does not claim reliable variable-tempo mapping for all
  music.
- yt-dlp acquisition is optional and requires an explicit URL workflow and
  network access. It is not needed to import local files.

## Localization and hosted administration

- Localization infrastructure and ten release catalogs exist, but the
  application is not fully localized. Most secondary screens, dialogs,
  inspector text, errors, help, and accessibility labels remain English;
  translations are not fully human-reviewed or visually certified.
- The optional hosted admin console reports Windows process/system CPU,
  process working-set memory, and process I/O counters. GPU/VRAM and physical
  disk throughput remain unavailable. Non-Windows hosts report these metrics
  as unavailable. Its process log ring is bounded, secret-redacted, and only
  retained in memory while the admin-enabled process is running; it is not a
  durable general logging archive. Render-job events remain durable with job
  history.

## Installation and validation

- The configured release workflow targets Windows x86_64 only. Linux and
  macOS are not packaged or certified. Windows installers are unsigned, and
  packaged assets still require a documented redistribution-rights review.
- FFmpeg/FFprobe are not bundled. A Windows first MP4 export may need network
  access to obtain the pinned FFmpeg build. This review used the shared local
  add-on pair, so it does not certify first-run download or offline clean
  installation.
- No clean-machine GUI, installed GUI, microphone, Whisper-model, yt-dlp
  acquisition, notification delivery, or full flagship workflow certification
  was available in this environment.
- Composition recovery checkpoints are atomic and the application now prompts
  before closing with unsaved Presentation Studio edits. An abrupt crash or
  power loss can still lose edits made since the most recent checkpoint.
- The application has no crash-bundle/safe-mode workflow or project file
  association. Uninstall intentionally keeps per-user application data.

See [the feature-existence audit](../FEATURE_EXISTENCE_AUDIT.md),
[installation guidance](INSTALLATION.md), and
[release-candidate certification](../RELEASE_CANDIDATE_CERTIFICATION.md) for
the supporting evidence and current release decision.
