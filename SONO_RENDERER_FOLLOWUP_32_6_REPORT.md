# Sono-Renderer Follow-up #32.6 — Presentation Studio NLE Completion

## Executive result

This pass extended the existing standalone composition/editor. It did not create a second editor or add a Sonolus runtime dependency to ordinary compositions. The production composition path now covers linked video/audio, still-image sources, generated color sources, source-range editing, track-scoped ripple delete, slip, a source monitor preview, and safer media-bin management. A real FFmpeg fixture validates standalone ordinary media, preview, audio mixing, save/reopen, and landscape/portrait MP4 export.

This is a verified foundational increment, **not completion of the full professional-NLE brief**. Major requested editing capabilities remain absent, including transitions, ordered effect stacks, masks, speed/reverse/freeze, arbitrary grouping, nesting, proxies, drag/drop, and export queue/history. The table below says so directly.

## Architecture found in the current repository

- `src/composition.rs` owns the standalone composition schema and media/track/clip model. A composition references media by stable source ID; clips refer to a source and carry timeline/source ranges, transforms, effects currently supported by the composition graph, text/audio properties, and scalar keyframes. Schema version 3 migrates versions 1 and 2 and rejects unknown future versions without rewriting them.
- Ordinary media remains independent of Sonolus. The composition module has no Watch, EngineData, LevelData, or Sono-GCC dependency. A pre-rendered Sonolus video enters through the same ordinary video-source path; live Watch output is not a composition source.
- FFmpeg probing, composition preview, source preview, and export use the existing composition model/filter graph. Preview/export are asynchronous editor jobs. Timeline playback uses the existing streaming decode/mix path and CPAL output; microphone capture is a separate explicit input path whose finalized WAV is ordinary project audio.
- `src/gui.rs` owns the current egui application and Presentation Studio UI. It also owns substantial unrelated GUI state, so it remains a maintainability risk; this pass did not mechanically split the file.
- Project serialization stores references and edits, not embedded media bytes. Save/recovery behavior remains in the existing composition/project path.

## Changes made in this pass

- Added/imported PNG and JPEG still sources with per-instance clip duration; stills use the same transform/crop/opacity/filter/keyframe/render graph as other visual clips.
- Added generated RGB solid sources that do not require external files.
- Added linked A/V clip pairs backed by one media source. Move, split, slip, delete, ripple, insert, and explicit unlink preserve or intentionally break the pair relationship.
- Added source in/out insertion modes: place, append, insert-and-ripple, and overwrite. Insert opens a sequence gap across tracks and shifts markers. Overwrite preserves non-overlapping tails and refuses to cut a linked pair until unlinked.
- Added source monitor selection/time/In/Out and a one-frame preview rendered by the composition FFmpeg graph, with actions for video, audio, and linked A/V ranges.
- Added track lock enforcement to composition edits and the clip inspector. Added track-scoped ripple delete and one-frame slip controls.
- Improved the media bin with name/path search, validated Replace/Relink, and Remove unused. Removing unused entries changes the project only; it never deletes files from disk.
- Updated Presentation Studio documentation and retained a small synthetic validation set under `artifacts/nle-validation/`.

## Real-media validation and visual inspection

Reproduce the real standalone fixture from the repository root when the app-managed FFmpeg/FFprobe pair is available:

```powershell
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture
```

The first integration test passed during this pass. It creates ordinary synthetic MP4/WAV/PNG/JPEG media, a linked video+audio source, generated solid, multiple tracks, keyframed transforms/blur, text, fades/gain/pan, saves and reopens the composition, obtains Program and Source-monitor frames, checks streamed video/PCM, and exports FFprobe-validated 1920×1080 and 1080×1920 MP4s. The second FFmpeg streaming-pipe test passed. Neither fixture constructs an engine, LevelData, Watch VM, gameplay event, Sono-GCC job, or Sonolus resource.

I visually inspected all four PNGs. The Source-monitor image is a nonblank full-frame test source. The still/solid frame contains the requested still and generated-color layers. The landscape and vertical images show their synthetic diagnostic layers; the vertical image uses a blurred fill behind its centered landscape foreground. The overlaps are busy and deliberately synthetic. These are evidence that the production compositor generated inspectable pixels, **not evidence of a polished visual design or a completed NLE GUI**. No human footage was included.

| Artifact | Composition time | Resolution | SHA-256 |
|---|---:|---:|---|
| [standalone-landscape-1920x1080.png](artifacts/nle-validation/standalone-landscape-1920x1080.png) | 0.75 s | 1920×1080 | `3a5c5ae0be4796746a0e12ed6815934e1bbd8dd81b46d00cd66f637cd3c8ecaa` |
| [standalone-vertical-1080x1920.png](artifacts/nle-validation/standalone-vertical-1080x1920.png) | 0.75 s | 1080×1920 | `cb6b20b876796ec46a9a9dec96ba0f2e1bc66d99d97b315614ea1e7d8f487910` |
| [standalone-png-jpeg-and-solid.png](artifacts/nle-validation/standalone-png-jpeg-and-solid.png) | 1.25 s | 1920×1080 | `cdde60c2ef09033ed1633543b6fcc35817c5e50742b575c2f09af8621f52221a` |
| [source-monitor-video-at-1.25s.png](artifacts/nle-validation/source-monitor-video-at-1.25s.png) | 1.25 s source time | 1920×1080 | `d7014ee4b4941a24bbb04590a7bb6960ca00df3ccb1409cc95d2f0de56c78f6b` |

The fixture details and its synthetic-media caveat are also in [artifacts/nle-validation/README.md](artifacts/nle-validation/README.md).

## Validation results

| Check | Result |
|---|---|
| Baseline before this pass: focused composition model tests | 15 passed, 1 ignored |
| Baseline before this pass: focused playback model tests | 2 passed, 1 ignored |
| Baseline before this pass: complete library tests | 314 passed, 6 ignored |
| Final `cargo fmt --all -- --check` | Passed |
| Final `cargo check --locked --all-targets` | Passed; build script warned that `rc.exe` is unavailable, so Windows executable version metadata was not embedded |
| Final normal `cargo test --locked` | Passed: library 324 passed / 6 ignored; main 101 passed; `watch_scheduler` 9 passed; doc tests 0 |
| Final serial `cargo test --locked -- --test-threads=1` | Passed: library, main, scheduler, and doc-test executables completed without a failure or process abort |
| Focused composition tests after final media-bin change | 26 passed, 1 ignored |
| Real FFmpeg composition preview/export fixture | Passed; ignored fixture invoked explicitly |
| Real FFmpeg continuous playback pipe fixture | Passed; ignored fixture invoked explicitly |
| `cargo build --locked --release` | Passed in 2m 41s; same `rc.exe` metadata warning |
| Strict Clippy (`cargo clippy --locked --all-targets -- -D warnings`) | Failed on the repository's existing warning backlog (164 warnings promoted to errors across unrelated older modules and tests); no repository-wide warning cleanup was attempted |
| Performance comparison / soak run | Not performed; the release compile duration is not a render benchmark |
| Interactive GUI validation | Not performed; compile/build checks do not establish mouse/keyboard usability |
| Physical audio-device / packaged-app validation | Not performed in this pass |

The full normal and serial suites both passed this time. The prior intermittent Windows process-abort history remains unresolved technical debt; these successful runs do not prove it cannot recur.

## Capability matrix

Status vocabulary is intentionally restricted to the requested labels. “Verified” means the stated production path and test evidence ran; a passing model test alone is not described as full GUI validation.

| # | Capability | Status | Production path, evidence, and limits |
|---:|---|---|---|
| 1 | Current composition architecture | IMPLEMENTED + VERIFIED | `CompositionProject` is independent of Watch/engine state; ordinary-media integration rendered with zero Sonolus content. |
| 2 | #32.5 baseline: multitrack, playback, audio, properties, keys, undo/recovery | IMPLEMENTED + NOT FULLY VERIFIED | Existing model/UI paths and tests exist; this pass did not interactively exercise the complete editor workflow. |
| 3 | Current capability/gap inspection | IMPLEMENTED + VERIFIED | This matrix is based on current code plus executed tests, not earlier prompt claims. |
| 4 | Linked video and audio | IMPLEMENTED + VERIFIED | One source, synchronized linked clip pair, unlink and linked edits; composition tests and real FFmpeg fixture. |
| 5 | PNG/JPEG still images | IMPLEMENTED + VERIFIED | Imported and rendered as timed visual clips in the real FFmpeg fixture. |
| 6 | Generated solid/color source | IMPLEMENTED + VERIFIED | Generated source goes through ordinary composition/render code; fixture rendered it without an external image file. |
| 7 | Media bin / project media | IMPLEMENTED + NOT FULLY VERIFIED | Shows kind, streams, dimensions, duration, use count; search, replace/relink, safe remove-unused. No bins/folders or reveal/open-location; UI not manually exercised. |
| 8 | Asynchronous cached thumbnails | NOT IMPLEMENTED | Bin rows have no generated/cached thumbnail pipeline. |
| 9 | Source monitor | IMPLEMENTED + NOT FULLY VERIFIED | Separate selected source/time/In/Out and normal-graph frame generation; real source PNG tested. Source audio playback/timecode and interactive monitor operation are absent. |
| 10 | Program monitor | IMPLEMENTED + NOT FULLY VERIFIED | Existing composition playback/seek and full-frame preview/export path; pipe/export tests passed. No interactive GUI or physical audio-output test. |
| 11 | Timeline ergonomics | IMPLEMENTED + NOT FULLY VERIFIED | Tracks/order, zoom, playhead, markers, snapping, locks and scrolling exist. No adjustable track height, multi-select/marquee, drag/drop, composition In/Out range, or snapping toggle. |
| 12 | Place/append/insert/overwrite/ripple/slip | IMPLEMENTED + NOT FULLY VERIFIED | Model operations and UI controls exist; insert, overwrite, ripple-delete and slip are tested. Ripple trim is absent; real mouse editing not exercised. |
| 13 | Arbitrary clip grouping | NOT IMPLEMENTED | Linked A/V is a distinct relationship; there is no user grouping model. |
| 14 | Nested/compound composition source | NOT IMPLEMENTED | No nested-composition evaluator or cycle rejection. |
| 15 | Cross-dissolve / dip transitions | NOT IMPLEMENTED | No transition data model. |
| 16 | Transition UI | NOT IMPLEMENTED | No transition controls or timeline representation. |
| 17 | Ordered effect stack | NOT IMPLEMENTED | Visual clip currently exposes a Gaussian blur property rather than a stable ordered stack. |
| 18 | Baseline effects | IMPLEMENTED + NOT FULLY VERIFIED | Gaussian blur is in production compositor and present in the real rendered fixture; brightness, contrast, saturation, hue, and sharpen are absent. |
| 19 | Effect copy/paste, reorder, bypass, presets | NOT IMPLEMENTED | No effect-stack editing model. |
| 20 | Rectangle/ellipse masks | NOT IMPLEMENTED | No mask model. |
| 21 | Mask compositing, feather, invert | NOT IMPLEMENTED | No mask renderer or acceptance frames. |
| 22 | Chroma key | NOT IMPLEMENTED | No keyer. |
| 23 | Source-time mapping | IMPLEMENTED + NOT FULLY VERIFIED | Source In/Out and slip alter the selected source window. There is no general retime map. |
| 24 | Speed, reverse, freeze frame | NOT IMPLEMENTED | No speed/reverse/freeze model; source range/slip do not provide these semantics. |
| 25 | Titles/text | IMPLEMENTED + NOT FULLY VERIFIED | Text clips support font, position, fill, outline/shadow, alignment, opacity/size animation; real FFmpeg text fixture rendered. No title templates/background box and GUI was not interacted with. |
| 26 | Shapes | IMPLEMENTED + NOT FULLY VERIFIED | Generated RGB solid is supported. Rectangle, ellipse, line, and transparent generated shapes are absent. |
| 27 | Adjustment layers | NOT IMPLEMENTED | No adjustment-layer source/compositing semantics. |
| 28 | Scalar keyframes/easing | IMPLEMENTED + NOT FULLY VERIFIED | Model and inspector support current animated properties/easing; model and FFmpeg animation fixture passed. No curve editor or keyframe clipboard. |
| 29 | Curve editor | NOT IMPLEMENTED | Only discrete scalar keyframes/easing choices. |
| 30 | Audio input meters | IMPLEMENTED + NOT FULLY VERIFIED | Existing mic input meter reports RMS/peak/clipping; no physical device tested in this pass. |
| 31 | Track mute/solo and clip gain/pan/fades | IMPLEMENTED + VERIFIED | Existing mixer path and integration fixture check mixed PCM and track audio controls; GUI operation not manually exercised. |
| 32 | Audio crossfades | NOT IMPLEMENTED | Independent fades exist; there is no explicit crossfade object/operation. |
| 33 | Audio processing/effects | IMPLEMENTED + NOT FULLY VERIFIED | Mix, gain, pan, and fades work in the common graph. No audio effect stack or common audio effects. |
| 34 | Proxy system | NOT IMPLEMENTED | No proxy generation, selection, invalidation, or original-quality export policy. |
| 35 | Preview/render cache | NOT IMPLEMENTED | No bounded decoded-frame/render cache acceptance path established in this pass. |
| 36 | Background workers | IMPLEMENTED + NOT FULLY VERIFIED | Preview/export, continuous playback, and microphone paths use workers/streaming; no long-session/stress test. |
| 37 | Relink/replace media | IMPLEMENTED + NOT FULLY VERIFIED | Probe validates candidate stream/type and existing clip compatibility before retaining source identity; dialog paths compile, but were not clicked through. |
| 38 | Media file identity | IMPLEMENTED + NOT FULLY VERIFIED | Sources use canonical/relative path references; no content hash identity for detecting same/mutated file at a different path. |
| 39 | Drag/drop import and timeline placement | NOT IMPLEMENTED | No editor drag/drop discovery path. |
| 40 | Markers/guides | IMPLEMENTED + NOT FULLY VERIFIED | Markers serialize and move during insert/ripple tests. No guide model. |
| 41 | Inspector | IMPLEMENTED + NOT FULLY VERIFIED | Existing inspector exposes current transform/text/audio/blur/keyframe fields and respects locked tracks. No coherent transition/effect-stack/mask/speed inspector. |
| 42 | Still-frame snapshot export | NOT IMPLEMENTED | Render Preview creates preview output; no standalone snapshot-save action was verified. |
| 43 | Export presets/settings | IMPLEMENTED + NOT FULLY VERIFIED | Output path and composition settings are configurable. No named codec/quality preset system; current MP4 path is fixed H.264/AAC. |
| 44 | Hardware encoding | NOT IMPLEMENTED | No hardware encoder selection/validation. |
| 45 | Export queue/history | NOT IMPLEMENTED | Presentation export is a single editor job, separate from a persistent NLE export queue/history. |
| 46 | Autosave/backups/recovery | IMPLEMENTED + NOT FULLY VERIFIED | Atomic project save and bounded recovery snapshots have model coverage; no GUI crash/restart acceptance run. |
| 47 | Performance/preview quality | IMPLEMENTED + NOT FULLY VERIFIED | Preview frame sizing and streaming are bounded; no NLE performance benchmark comparison was run. |
| 48 | Long-session stability | IMPLEMENTED + NOT FULLY VERIFIED | Worker/cancellation paths exist; no multi-hour soak or memory-growth test. |
| 49 | Windows parallel-test instability | IMPLEMENTED + NOT FULLY VERIFIED | Both normal and serial full suites passed this pass. Earlier intermittent process abort remains unresolved; this is not a fix claim. |
| 50 | Packaged-application validation | IMPLEMENTED + NOT FULLY VERIFIED | Release binary build passed. Packaged layout/media probing/playback/export was not exercised. |
| 51 | Platform support | IMPLEMENTED + NOT FULLY VERIFIED | Windows build and tests ran. Linux/macOS runtime behavior is not established by this evidence. |
| 52 | Normal-video acceptance A | IMPLEMENTED + NOT FULLY VERIFIED | Synthetic 1920×1080 fixture covers ordinary multi-source media, stills, title, tracks, audio, linked A/V, edits, blur/keyframes, save/reopen/playback/export. Missing transition, extra effects, mask, and human GUI acceptance prevent full pass. |
| 53 | Vertical Short acceptance B | IMPLEMENTED + NOT FULLY VERIFIED | Synthetic 1080×1920 composition/export and reframed blurred background were tested. No transition; creator workflow not interactively exercised. |
| 54 | Voiceover acceptance C | IMPLEMENTED + NOT FULLY VERIFIED | Existing WAV/audio timeline integration is exercised; mic backend code exists. Physical capture, waveform UI, and voiceover editing acceptance were not performed. |
| 55 | Sonolus render imported as media D | IMPLEMENTED + NOT FULLY VERIFIED | Generic video source has no Sonolus branch and accepts ordinary MP4s. This pass did not import a real Sono-Renderer output. |
| 56 | Nesting acceptance | NOT IMPLEMENTED | No child/parent composition support to validate. |
| 57 | Proxy acceptance | NOT IMPLEMENTED | No proxy support to validate. |
| 58 | Effects/masks visual evidence | IMPLEMENTED + NOT FULLY VERIFIED | PNGs visibly exercise blur and transforms. No ordered effects, masks, inversion, or feather exist. |
| 59 | Time-mapping visual/equivalence evidence | NOT IMPLEMENTED | No speed/reverse/freeze feature to compare. |
| 60 | MP4 export acceptance | IMPLEMENTED + VERIFIED | FFmpeg produced H.264/AAC landscape and portrait outputs; FFprobe validated streams, dimensions, duration, and audio. |
| 61 | Save/reopen acceptance | IMPLEMENTED + VERIFIED | Integration fixture saved/reopened composition and checked sources, linked clips, and still metadata before rendering/export. Advanced absent features cannot be round-tripped. |
| 62 | Undo/redo torture sequence | IMPLEMENTED + NOT FULLY VERIFIED | Existing snapshot undo/redo is wired to editing; no full import-edit-effect-transition-delete torture workflow was performed. |
| 63 | Failure acceptance | IMPLEMENTED + NOT FULLY VERIFIED | Tests cover invalid schema, missing/invalid inputs, lock atomicity, and export cleanup in existing suite. No full failure matrix for thumbnails/proxies/masks/encoders because those systems are absent. |
| 64 | Renderer regression / Watch invariant | IMPLEMENTED + VERIFIED | Full normal and serial renderer tests passed; scheduler integration passed. This does not replace visual/audio real-device regression or resolve historical abort reports. |

## Limitations and intended next work

The largest architecture gap is that visual effects remain clip fields rather than a typed ordered stack. Transitions, masks, nesting, grouping, retiming, proxies, and multi-selection should be added only as real serialized composition semantics consumed by both preview and export, not as UI-only options. The next practical completion slice should establish a typed effect stack and a simple transition model, then validate both preview/export against extracted real frames before widening the feature surface.

The timeline still lacks ripple trim, composition In/Out, drag/drop, multi-selection, and track-height control. Media management still lacks thumbnails, bins/folders, and reveal/open-location. No interactive GUI or packaged artifact was tested. Performance, long-session stability, A/V drift, and physical microphone latency remain unmeasured. Strict Clippy remains blocked by the repository's wider warning backlog.

## Reproduction commands

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked
cargo test --locked -- --test-threads=1
cargo test --locked --lib composition::tests
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture
cargo build --locked --release
```
