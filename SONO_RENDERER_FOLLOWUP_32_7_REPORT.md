# Sono-Renderer Follow-up #32.7 — Presentation Studio NLE completion wave 2

## Result

This pass extended the existing standalone composition/editor and filter graph. It did not introduce a second project, timeline, media, preview, or effect architecture. The composition schema is now version 7. Existing transitions and constant source-time fields migrate from prior versions; schema 6 migrates to schema 7 without altering existing effects. Ordinary media compositions remain standalone and do not initialize Watch, Sono-GCC, EngineData, LevelData, or Sonolus resources.

Implemented and validated in this pass:

- First-class cross-dissolve, dip-to-black, and dip-to-white transition records, with model operations, persistence, timeline/inspector controls, and production compositor output.
- Rectangle/ellipse masks with normalized bounds, feather, invert, keyframes, inspector controls, and production compositor output.
- Typed, ordered, bypassable per-clip visual effects, including chroma key and gamma; gamma uses the same stack and supports 0.1–10.0.
- Constant source-time mapping for forward 0.1×–8×, reverse, and freeze; duration remains timeline time. Linked A/V receives the same mapping. Audio uses FFmpeg `atempo` (therefore pitch changes), `areverse`, or silence for freeze.
- Retimed split behavior for forward/reverse source windows and linked A/V consistency checks.
- Real-render comparisons for effect order and bypass, plus real mask/effect, transition, and source-clock output evidence.

The remaining work is substantial. Variable-speed ramps, grouping, nested compositions, thumbnails, proxies, render/frame cache, curve editor, adjustment layers, drag/drop, advanced export, and several other ordinary editor capabilities remain absent. This is not a claim that the complete #32.7 brief or a professional NLE acceptance edit is finished.

## Implementation and ownership

`src/composition.rs` remains authoritative for the versioned project schema, model validation, source/clip/transition/effect state, editing operations, keyframes, preview, and export filter graph. `src/gui.rs` exposes the currently implemented mask, effect, speed, and transition controls in the existing Presentation Studio inspector/timeline. Preview and MP4 export share the composition graph. Save/reopen continues to use the existing composition serialization and undo continues to snapshot the same project model.

Transition IDs refer to two overlapping visual clips on the same track; the incoming clip must be above the outgoing clip. Cross-dissolve ramps incoming opacity while retaining the outgoing image underneath. Dip transitions add a full-frame black/white overlay with a triangular opacity envelope. Clip motion keeps a transition inside the remaining overlap or rejects an invalid move. A split through a transition is rejected until the transition is removed/repositioned.

Masks are applied after the effect stack and before clip opacity/transform. Their coordinates are normalized against the cropped/scaled source image. Mask scalar parameters use the existing generalized clip keyframe/easing representation. Freeform/Bezier masks are not implemented.

Source-time mapping separates timeline duration from consumed source duration. Forward and reverse consume `duration × playback_rate`; reverse stores the low source-window boundary and reads the window backwards. Freeze samples at `source_start`. The current rate is constant per clip, not a ramp. Audio tempo changes pitch, reverse audio is reversed, and frozen-video audio is silent. Preview/export use the same graph. The reverse filter may buffer the full source clip.

Gamma is a new `VisualEffectKind` in the existing serialized effect enum. It has finite-range validation and an inspector slider. Schema 7 marks that enum expansion so older builds reject it as a future project rather than interpreting a schema-6 document containing an unknown effect kind.

## Validation performed

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed after the final schema-7 migration patch |
| `cargo check --locked --all-targets` | Passed after the final schema-7 migration patch |
| Focused `composition::tests` | 34 passed, 1 ignored after the final schema-7 migration patch |
| Transition model test | Passed: persistence, overlap, add/update/remove, graph construction |
| Mask model test | Passed: validation and generalized animated properties |
| Source-time model test | Passed: FFmpeg graph has explicit source trim/tempo/reverse/freeze behavior |
| Retimed split/link test | Passed: forward/reverse split windows and linked A/V mode/rate stay aligned |
| Real FFmpeg integration | Passed after schema 7: `real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export` with `--ignored --nocapture`, 48.39 s. It saves/reopens the composition, renders masks, transitions, effect ordering/bypass and source-time frames, exports reverse-mapped/landscape/vertical MP4s, and FFprobe-validates outputs. |
| Visual inspection | Inspected the 3 new effect-stack frames and existing mask/transition/source-time/main frames. The two effect orders are visibly different, bypass returns to the source color, mask boundaries are clear, transition midpoints show expected blends/colors, and test-pattern clocks show source mapping. Main synthetic landscape/vertical frames are busy and overlap intentionally; they are diagnostic fixtures rather than polished edit examples. |
| Interactive GUI | Not performed. `cargo check` establishes compilation only. |
| Physical audio, packaged app, long-session/drift, proxy/nesting | Not tested; corresponding features or environment acceptance remain incomplete. |
| Full parallel `cargo test --locked` | Passed before the final schema-7-only migration patch: 472 passed, 6 ignored across library, main, integration, scheduler and doc-test targets. |
| Full serial `cargo test --locked -- --test-threads=1` | Passed before the final schema-7-only migration patch: 472 passed, 6 ignored. Library serial portion took 30m37s; all targets completed without process abort. |
| `cargo build --locked --release` | Passed after the final schema-7 migration patch; 1m50s. `rc.exe` was unavailable, so Windows executable version metadata was not embedded. |

The ignored fixture requires the application's already-installed FFmpeg/FFprobe pair and does not download tools. Its outputs and hashes are documented in [the artifact README](artifacts/nle-validation/README.md). Hashes identify generated files, not visual correctness.

## Updated #32.6 capability matrix

Statuses distinguish feature presence from complete acceptance. Paths below refer to production code unless described as tests/artifacts. GUI controls were build-checked but not interactively exercised in this environment.

| # | Requirement | Status | Evidence and remaining boundary |
|---:|---|---|---|
| 1 | Standalone composition architecture | IMPLEMENTED + VERIFIED | `CompositionProject` and ordinary FFmpeg source path; zero-Sonolus real fixture. |
| 2 | #32.5 multitrack/playback/audio/keys/undo/recovery baseline | IMPLEMENTED + NOT FULLY VERIFIED | Existing model/UI paths; complete GUI workflow not manually exercised. |
| 3 | Capability inspection | IMPLEMENTED + VERIFIED | This matrix is derived from current code and run evidence. |
| 4 | Linked video/audio | IMPLEMENTED + VERIFIED | Shared source/link IDs; model and real-media fixture. |
| 5 | PNG/JPEG stills | IMPLEMENTED + VERIFIED | Imported and rendered in real FFmpeg fixture. |
| 6 | Generated solid source | IMPLEMENTED + VERIFIED | Ordinary generated source rendered without a media file. |
| 7 | Media bin | IMPLEMENTED + NOT FULLY VERIFIED | Metadata, search, replace/relink, remove-unused; no bins/reveal-location; GUI not manually used. |
| 8 | Async cached thumbnails | NOT IMPLEMENTED | No thumbnail worker/cache. |
| 9 | Source monitor | IMPLEMENTED + NOT FULLY VERIFIED | Isolated production-graph frame evidence; interactive source audio/time workflow absent/unverified. |
| 10 | Program monitor | IMPLEMENTED + NOT FULLY VERIFIED | Preview/export graph and playback pipe tests; no interactive GUI/device test. |
| 11 | Timeline ergonomics | IMPLEMENTED + NOT FULLY VERIFIED | Track ordering, ruler, zoom, snapping behavior, locks, markers; no multi-select, adjustable height, drag/drop, In/Out range, or snapping toggle. |
| 12 | Place/append/insert/overwrite/ripple/slip | IMPLEMENTED + NOT FULLY VERIFIED | Model operations and tests; retimed split covered. Full interactive edit and all linked retime slip cases not exercised. |
| 13 | Arbitrary grouping | NOT IMPLEMENTED | Linked A/V remains a different relationship; no user group model. |
| 14 | Nested composition | NOT IMPLEMENTED | No nested source evaluator/cycle rejection. |
| 15 | Dissolve/dip transitions | IMPLEMENTED + NOT FULLY VERIFIED | Transition model and real pixel frames; mouse/keyboard workflow not exercised. |
| 16 | Transition timeline/inspector UI | IMPLEMENTED + NOT FULLY VERIFIED | Existing GUI code compiles; no interactive validation. |
| 17 | Ordered effect stack | IMPLEMENTED + NOT FULLY VERIFIED | Stable IDs, order, bypass, typed stack, GUI; unit and real pixel order comparison. Effect parameter keyframing is absent. |
| 18 | Baseline effects | IMPLEMENTED + NOT FULLY VERIFIED | Blur, brightness, contrast, saturation, hue, sharpen, gamma, chroma key; real stack/chroma pixels. Not every individual filter has a dedicated artifact; keyer has no spill suppression. |
| 19 | Effect copy/paste/reorder/bypass/presets | IMPLEMENTED + NOT FULLY VERIFIED | Copy/paste, stack copy/paste, reorder, bypass implemented; presets are absent; GUI not interacted with. |
| 20 | Rectangle/ellipse masks | IMPLEMENTED + NOT FULLY VERIFIED | Model, keyframe integration and real frames. Freeform absent; GUI build only. |
| 21 | Mask feather/invert/compositing | IMPLEMENTED + NOT FULLY VERIFIED | Real pixel checks for hard/feathered/inverted/animated; no freeform mask. |
| 22 | Chroma key | IMPLEMENTED + NOT FULLY VERIFIED | Typed key color/tolerance/softness and real pixel-difference assertion; baseline FFmpeg quality only. |
| 23 | Source-time mapping | IMPLEMENTED + NOT FULLY VERIFIED | Timeline/source duration model, preview/export graph, source-clock frames; GUI not interactive. |
| 24 | Speed/reverse/freeze | IMPLEMENTED + NOT FULLY VERIFIED | 0.1×–8×, reverse and freeze; real previews and reverse MP4. Audio pitch changes; no drift/long-clip test. |
| 25 | Titles/text | IMPLEMENTED + NOT FULLY VERIFIED | Text, font, fill, outline, shadow, alignment and animated fields; no background box or title template; GUI not manually checked. |
| 26 | Shapes | IMPLEMENTED + NOT FULLY VERIFIED | Generated RGB solids only; rectangle/ellipse/line vector shape clips absent. |
| 27 | Adjustment layers | NOT IMPLEMENTED | No effect-on-composited-below layer semantics. |
| 28 | Scalar keyframes/easing | IMPLEMENTED + NOT FULLY VERIFIED | Generalized property model and easing; masks included. No curve view/clipboard; effect-stack params remain static. |
| 29 | Curve editor | NOT IMPLEMENTED | Discrete keys/inspector only. |
| 30 | Microphone level meter | IMPLEMENTED + NOT FULLY VERIFIED | Existing input RMS/peak/clipping path; no physical input device test in this pass. |
| 31 | Audio mute/solo/gain/pan/fades | IMPLEMENTED + VERIFIED | Common mix graph plus model/real fixture evidence. |
| 32 | Audio crossfades | NOT IMPLEMENTED | Independent fades exist; no explicit crossfade workflow/object. |
| 33 | Audio effects/analysis | IMPLEMENTED + NOT FULLY VERIFIED | Ordinary mix/gain/pan/fades exist; no audio effect stack, normalization or master meter acceptance. |
| 34 | Proxy system | NOT IMPLEMENTED | No proxy lifecycle or original-only final-export policy. |
| 35 | Preview/frame cache | NOT IMPLEMENTED | No bounded render cache; no profiling-backed cache addition in this pass. |
| 36 | Background workers | IMPLEMENTED + NOT FULLY VERIFIED | Preview/export/playback/recording use workers; no NLE long-session stress run. |
| 37 | Relink/replace media | IMPLEMENTED + NOT FULLY VERIFIED | Candidate probing/compatibility checks and source ID retention; GUI path only build-checked. |
| 38 | Media identity | IMPLEMENTED + NOT FULLY VERIFIED | Portable relative/absolute path semantics; no content identity at alternate paths. |
| 39 | Drag/drop | NOT IMPLEMENTED | No editor file/source drag-drop workflow. |
| 40 | Markers/guides | IMPLEMENTED + NOT FULLY VERIFIED | Marker persistence/movement tested; guide model absent. |
| 41 | Unified inspector | IMPLEMENTED + NOT FULLY VERIFIED | Transform/audio/text/effects/mask/speed/transition controls compile; interactive path unverified. |
| 42 | Snapshot save | NOT IMPLEMENTED | Render Preview exists; no verified save-still action. |
| 43 | Export presets/settings | IMPLEMENTED + NOT FULLY VERIFIED | Resolution/FPS/project output settings and MP4 path exist; no named quality/codec preset system. |
| 44 | Hardware encoder selection | NOT IMPLEMENTED | No verified capability discovery/selection. |
| 45 | Composition export queue/history | NOT IMPLEMENTED | Exports are not integrated into generic persistent job/history infrastructure. |
| 46 | Autosave/recovery | IMPLEMENTED + NOT FULLY VERIFIED | Atomic save and bounded recovery snapshots; no crash/restart GUI acceptance. |
| 47 | Preview quality/performance | IMPLEMENTED + NOT FULLY VERIFIED | Bounded preview dimensions/streaming; no NLE benchmark. |
| 48 | Long-session stability | IMPLEMENTED + NOT FULLY VERIFIED | Worker/cancellation foundations exist; no soak/memory-growth run. |
| 49 | Windows parallel-test instability | IMPLEMENTED + NOT FULLY VERIFIED | A clean run is recorded below if it completes; historical intermittent process abort is not thereby fixed. |
| 50 | Packaged application | IMPLEMENTED + NOT FULLY VERIFIED | Release build evidence only; package layout and media workflow not exercised. |
| 51 | Platform support | IMPLEMENTED + NOT FULLY VERIFIED | Windows build/run only; other OS behavior unestablished here. |
| 52 | Normal-video acceptance A | IMPLEMENTED + NOT FULLY VERIFIED | Real ordinary-media landscape edit/export fixture covers sources, stills, tracks, audio, linked A/V and several edits. Missing grouping/nesting/curve and GUI acceptance prevent full pass. |
| 53 | Vertical acceptance B | IMPLEMENTED + NOT FULLY VERIFIED | Real 1080×1920 fixture/export and blurred fill; no finished multi-clip creator workflow. |
| 54 | Voiceover acceptance C | IMPLEMENTED + NOT FULLY VERIFIED | Input/media code exists; physical capture, monitor, waveform editing were not tested here. |
| 55 | Sonolus video-as-media D | IMPLEMENTED + NOT FULLY VERIFIED | Ordinary MP4 path is generic; no real renderer output imported in this pass. |
| 56 | Nesting acceptance | NOT IMPLEMENTED | Depends on missing nested composition source. |
| 57 | Proxy acceptance | NOT IMPLEMENTED | Depends on missing proxy system. |
| 58 | Effect/mask visuals | IMPLEMENTED + VERIFIED | Real generated frames cover effect order/bypass, mask shape/feather/invert/animation and chroma-key pixel changes. |
| 59 | Time-map visuals/equivalence | IMPLEMENTED + NOT FULLY VERIFIED | Real source-clock previews and reverse export; audio tempo/pitch and long duration sync not measured. |
| 60 | MP4 export | IMPLEMENTED + VERIFIED | FFmpeg/FFprobe acceptance fixture validates landscape/vertical output streams, duration and dimensions. |
| 61 | Save/reopen | IMPLEMENTED + VERIFIED | Integration round trip checks media, linked clips and still metadata before rendering. |
| 62 | Undo/redo torture workflow | IMPLEMENTED + NOT FULLY VERIFIED | Snapshot undo/redo exists; no interactive whole-editor torture sequence. |
| 63 | Failure acceptance | IMPLEMENTED + NOT FULLY VERIFIED | Model validation and export cleanup tests; no device/proxy/nesting failure matrix. |
| 64 | Renderer/Watch regression | IMPLEMENTED + NOT FULLY VERIFIED | Full-suite result below; no claim that NLE changes replace Watch semantic regression or prove historical abort resolved. |

## #32.7 wave matrix

| Wave | Status | Production path and evidence | Remainder |
|---|---|---|---|
| A — Ordered effects | IMPLEMENTED + NOT FULLY VERIFIED | `VisualEffectInstance` typed stack/order/bypass, gamma; model and pixel order/bypass tests. | Effect parameter keyframes and presets absent; GUI not interactive. |
| B — Transitions | IMPLEMENTED + NOT FULLY VERIFIED | Persistent transition model, validation/edit operations, timeline/inspector; real crossfade/black/white pixel fixtures. | Interactive GUI not tested. |
| C — Masks | IMPLEMENTED + NOT FULLY VERIFIED | Rectangle/ellipse/feather/invert, generalized scalar keys; actual FFmpeg pixel assertions/frames. | Freeform/Bezier mask deferred as unsupported. |
| D — Chroma key | IMPLEMENTED + NOT FULLY VERIFIED | Typed color/tolerance/softness effect; actual pixel-difference assertion in ordinary compositor. | No spill suppression/pro key quality. |
| E — Source time | IMPLEMENTED + NOT FULLY VERIFIED | Constant forward/reverse/freeze, source windows, linked A/V and preview/export; real test-pattern frames and reverse MP4. | Pitch-preserving audio and drift testing absent. |
| F — Variable speed | NOT IMPLEMENTED | Constant `playback_rate` only. | No ramp representation/UI or audio time-varying retime. |
| G — Nested compositions | NOT IMPLEMENTED | No child composition source. | No recursive dependency/cycle handling. |
| H — Grouping | NOT IMPLEMENTED | Linked A/V only. | No user-selectable group IDs or multi-select workflow. |
| I — Slip editing | IMPLEMENTED + NOT FULLY VERIFIED | One-frame source shift, linked sync, retimed-aware delta scaling in UI; model tests. | No manual GUI validation or dedicated retimed slip edge-bound tests. |
| J — Media bin | IMPLEMENTED + NOT FULLY VERIFIED | Existing canonical source metadata/search/relink/replace/remove-unused. | No folders/reveal location; UI not exercised. |
| K — Thumbnails | NOT IMPLEMENTED | No async thumbnail cache. | — |
| L — Timeline feedback | IMPLEMENTED + NOT FULLY VERIFIED | Selection, clip labels, link/effect/animation indications, waveform, tracks/markers/snapping. | No group/proxy badges, drag/drop, snapping toggle, composition In/Out, adjustable heights. |
| M — Proxies | NOT IMPLEMENTED | No proxy state/generation/selection. | Original-only export invariant not applicable yet. |
| N — Preview cache | NOT IMPLEMENTED | No bounded frame cache. | No performance profiling performed to justify cache complexity. |
| O — Titles/graphics | IMPLEMENTED + NOT FULLY VERIFIED | Text and font/fill/outline/shadow support; generated RGB solids. | Title background, templates, editable ellipse/rectangle/line shape clips absent. |
| P — Adjustment layers | NOT IMPLEMENTED | No adjustment layer compositing. | — |
| Q — Curve editor | NOT IMPLEMENTED | Existing keys/easing remain the only editor. | No graph, key movement/copy/paste UI. |
| R — Audio maturation | IMPLEMENTED + NOT FULLY VERIFIED | Gain, mute/solo, pan, fades, common mixer; source-time audio behavior explicit. | No crossfade operation/master meter/clipping workflow or effect chain acceptance. |
| S — Export maturation | IMPLEMENTED + NOT FULLY VERIFIED | Existing resolution/FPS and H.264/AAC export, landscape/vertical FFprobe validation. | No named presets, quality controls, range export, or hardware encoder selection. |
| T — Export queue/history | NOT IMPLEMENTED | No generic composition-export queue adapter. | Sonolus render jobs are not suitable to represent generic compositions. |
| U — Recovery | IMPLEMENTED + NOT FULLY VERIFIED | Atomic save and bounded recovery checkpoint model/UI code. | No GUI crash/relaunch test in this run. |
| V — Drag/drop | NOT IMPLEMENTED | No file/source drop mapping. | — |
| W — Inspector | IMPLEMENTED + NOT FULLY VERIFIED | Existing inspector now exposes implemented transition/effect/mask/speed fields. | GUI build evidence only; no interactive selection/edit/undo exercise. |

## #32.7 real-world acceptance matrix

| Acceptance | Status | Evidence and gap |
|---|---|---|
| A — Normal video | IMPLEMENTED + NOT FULLY VERIFIED | Ordinary-media 1920×1080 production preview/export, linked A/V, multiple tracks, stills, generated solids, text/audio, save/reopen; plus transition/effect/mask/time map real fixtures. Group, nested composition, curve editor and interactive end-to-end edit remain missing. |
| B — Vertical | IMPLEMENTED + NOT FULLY VERIFIED | 1080×1920 preview and FFprobe-validated MP4 with reframed source/blurred background. No finished title/animation/transition creator workflow through the GUI. |
| C — Nesting | NOT IMPLEMENTED | Nested source and cycle guards do not exist. |
| D — Proxy | NOT IMPLEMENTED | Proxy generation/preview/original-quality export do not exist. |
| E — Effects | IMPLEMENTED + NOT FULLY VERIFIED | Pixel frames prove ordered effects, bypass, masks/feather/invert, transitions and source-time modes. Nested composition has no implementation; GUI was not interactively exercised. |

## Reproduction and remaining risks

Run model/build validation from the repository root:

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked --lib composition::tests
cargo test --locked --lib timeline_playback::tests
cargo test --locked
cargo test --locked -- --test-threads=1
cargo build --locked --release
```

Run the real production-graph test when app-managed FFmpeg/FFprobe are present:

```powershell
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
```

Known evidence limits: no interactive GUI operation, no physical input/output device test, no packaged application test, no long-duration drift/soak test, no variable-speed audio test, no proxy/nesting/grouping acceptance, and no real Sonolus gameplay source. Windows produced the existing build warning that `rc.exe` was unavailable; executable version metadata was not embedded. Passing a parallel test run does not prove the previously reported intermittent Windows process abort is resolved.
