# Sono-Renderer Follow-up #36 — Feature Existence Audit

**Audit date:** 2026-10-08  
**Repository:** current `Sono-Renderer` worktree on Windows x86_64  
**Purpose:** distinguish source implementation, product reachability, persistence, real-input evidence, and packaged-artifact evidence. This report is a snapshot of the inspected tree and tests; it does not treat earlier prompts or reports as proof.

## Verdict

The current repository contains a substantial working renderer and standalone composition editor. The strongest evidence is the complete serial test suite, a real Next Sekai/Larp MP4 render, the production FFmpeg composition test, and a portable-package CLI smoke test built from the current release executable. Those results verify production boundaries; they do **not** certify interactive GUI usability, real microphone capture, real-speech recognition, live URL acquisition, or an installed Windows GUI.

Important implementation limits remain. The generic Sonolus runtime does not implement every v1.1.4 operation: stack operations and replay-stream provisioning remain incomplete, and curved Draw mesh rasterization is absent. Presentation Studio deliberately consumes pre-rendered gameplay as ordinary video; it has no live Watch source. Advanced NLE capabilities such as freeform masks, cubic-Bezier curve editing, and dedicated roll/slide tools are also absent. These are not relabeled as complete.

No production source code was changed during this pass. No newly discovered, narrowly scoped bug justified changing renderer semantics. The requested audit artifact, a README index link, and a dated validation addendum to the #35 report are the only repository edits. This avoids changing VM, Watch, rendering, audio, or project behavior without a demonstrated defect.

## How to read the matrix

- **Exists** means a production implementation was found, not merely a type or name.
- **Wired** means a current GUI or CLI path reaches it.
- **Persisted** means user-owned state survives the subsystem's save/reopen path; `N/A` means the operation is intentionally transient.
- **Real test** distinguishes production-boundary tests using real engines/media from unit-only or synthetic tests. It does not mean a human completed the GUI workflow.
- **Packaged** records evidence against a portable ZIP made from the current release executable. The smoke test exercised CLI startup/help/state paths only; it did not run a packaged GUI render.
- `Partial` is used where an implementation exists but the requested real-world behavior or validation remains incomplete. `No` means no evidence for that column.

## Master feature matrix

The matrix contains 37 capability rows. A companion trace below maps the earlier requested capability families to the current implementation owners; prompt prose is treated as intent, while the evidence columns refer to code paths and executions.

| Capability | Exists | Wired | Persisted | Real test evidence | Packaged evidence | Current status / boundary |
|---|---:|---:|---:|---|---|---|
| Engine/package, resource, and LevelData loading | Yes | Yes | N/A | Yes: supplied Next Sekai, Next RUSH, Horizon, SCP, and gzip LevelData tests; Larp render | CLI startup only | Implemented for recognized formats; unsupported payloads fail with errors |
| Classic engine + level + resources + music → MP4 | Yes | Yes | Config/project optional | Yes: actual 2 s Larp render; FFprobe reports H.264/AAC, 320×180, 6 fps, 12 frames | Partial: package CLI smoke; no export run from package | Core direct workflow works without project/profile/NLE |
| One authoritative Watch traversal | Yes | Yes | N/A | Yes: real-render and Watch integration tests; test suite asserts one playback traversal | No | `FrameSession`/`WatchRuntime` owns playback; diagnostics, preview, compatibility, and GCC do not define a second semantic runtime |
| Watch VM semantic path | Yes | Yes | N/A | Yes: Watch/m0/scheduler suites, supplied-engine initialization, CPU/WGPU comparisons | No | VM is default/oracle; incomplete v1.1.4 operations remain listed below |
| Sono-GCC | Yes | Yes | Yes: exact-graph native cache | Unit/differential tests pass; this audit's real Larp render used VM, not GCC | Partial: help/CLI smoke only; native compiler toolchain not exercised in package | Optional Windows native scalar regions with VM fallback; no speed claim |
| Particles | Yes | Yes | Config/profile data | Yes: production compositor unit tests and supplied engine/resource tests | No | Renderer path exists; no separate new particle-specific full render was run in this audit |
| SFX and BGM audio | Yes | Yes | Config/job inputs | Yes: audio/mixer tests and Larp MP4 with AAC stream; Larp 0–2 s had scheduled loops but no one-shot audio events | No | Offline decode/mix/mux; no real output-device playback test |
| Gameplay CPU/WGPU draw pipeline | Yes | Yes | Backend is config | Yes: WGPU/CPU comparison tests and included real-engine initialization/render tests | No | WGPU path exists; this audit did not identify the physical adapter or certify a discrete GPU |
| Gameplay UI profiles and asset packs | Yes | Yes | Yes: profile/config files | Unit/pixel tests cover multi-aspect layout, bitmap glyphs, events, and gauges; no real client oracle in this pass | No | Profile-driven visual HUD; engine-specific artwork remains data, not generic renderer behavior |
| LIFE / SCORE / COMBO / judgement state | Yes | Yes at presentation API | Profile/event data | Unit tests cover eased/negative LIFE, event score, discrete combo, judgement mapping, and direct-time evaluation | No | External event adapter is not a universal authoritative Sonolus score feed; synthetic events are not real gameplay results |
| Shared property/easing system across gameplay UI and NLE | Partial | Yes within each editor | Yes in profile/composition | Unit tests cover each domain | No | Domain-specific track data remains separate (`UiTimelineTrack` vs composition `ScalarKeyframe`); common easing semantics do not constitute one universal typed-property editor |
| NLE text / image / shape composition | Yes | Yes | Yes: `.sonocomp` | Yes: FFmpeg production test imports ordinary MP4/WAV/PNG/JPEG, generated geometry, and titles | Partial: portable CLI smoke only | Unicode parsing/edit tests; diagnostic NLE frames inspected, but not a designed visual theme review |
| NLE effects | Yes | Yes | Yes | Yes: actual FFmpeg filter graph tests for blur/color/chroma key/sharpen/gamma paths, ordering, keyframes, and export parity | Partial | Implemented effects are documented; blur/sharpen/chroma-key parameters are static, and keyframeable effects are a subset |
| NLE masks / clipping | Partial | Yes | Yes | Yes: real compositor test for rectangle/ellipse, invert, feather, and animated mask | Partial | No freeform/Bezier mask |
| NLE transitions | Yes | Yes | Yes | Yes: real FFmpeg dissolve/dip frames, trimmed/retimed overlap, and preview/export pixel comparison | Partial | First-class overlap data/UI; split through a transition is rejected until it is removed/repositioned |
| NLE timeline editing | Yes | Yes | Yes | Model tests for selection/group/linked A/V, trim, split, move, slip, ripple, insert, overwrite, locks, snap, undo/redo | Partial | No dedicated roll/slide edit tools; egui gestures were not manually driven |
| NLE media bin / import / relink | Yes | Yes | Yes | Yes: real video/audio/still import and media-bin thumbnail test | Partial | Bounded discovery/relink, media identities, generated/still sources; no manual drag gesture test |
| NLE proxies / preview cache | Yes | Yes | Yes | Yes: real proxy generation/preview/export fixture; final export uses original media | Partial | Proxy validity is tied to file signatures and metadata; not an automatic network service |
| NLE continuous preview / playback | Yes | Yes | Playhead/project state | Yes: FFmpeg decode-pipe test streams video/PCM and preserves filter files | Partial | Playback clock uses CPAL output sample frames; no physical output device, long-run drift, or sustained dropped-frame measurement |
| NLE audio mixing/editing | Yes | Yes | Yes | Yes: production graph checks WAV/video audio, gain/RMS, fades, crossfades, mute/solo/pan behavior | Partial | Monitoring/output device and long-session sync remain unverified |
| Standalone NLE independence | Yes | Yes | Yes | Yes: production FFmpeg fixture creates/edits/exports an ordinary-media composition without Sonolus inputs | Partial | Live Watch is intentionally not a composition source; rendered gameplay MP4 is ordinary media |
| Vertical/Shorts composition | Yes | Yes | Yes | Yes: production fixture exports/FFprobe-validates a 1080×1920 NLE composition from landscape sources | Partial | The exact historical multi-instance gameplay/0.32 spatial-scale/caption acceptance was not run; no separate Shorts renderer exists |
| Chart Studio | Yes | Yes | Yes: render-project extension and gzip LevelData | Yes: canonical LevelData/export/runtime tests and supplied engine fixtures | Partial | Generic raw entity editing and optional engine-bound schema; no spatial placement canvas or full GUI round-trip into a rendered MP4 was performed |
| BPM analysis | Yes | Yes | Applied grid stored with chart project | Yes: generated 120 BPM MP3 decode/playback alignment, fractional tempo, silence/noise/phase tests | Partial | No labeled real-song corpus; proposal requires explicit Apply and does not mutate Watch semantics |
| Microphone capture | Yes | Yes | Final WAV and audio clip persist | Unit tests for safe enumeration, writer/finalize/recovery, and ordinary clip integration | No real-device/package capture | Real capture, permissions, hot-plug, latency/drift, and monitoring were unavailable to verify; speaker monitoring is not implemented |
| Automatic subtitles | Yes | Yes | Yes: ordinary text clips in composition | SRT/VTT/Unicode/edit tests and real FFmpeg trim/rate→16 kHz mono extraction test | No model/package inference | No real speech was transcribed: whisper.cpp runtime and model were not available. Local model installation is explicit; no cloud upload |
| yt-dlp media acquisition | Yes | Yes (GUI and CLI) | Download becomes local file/project input | Validation and subprocess-argument tests; no real URL acquisition | Packaged tool-info reports unavailable | Current managed yt-dlp executable is absent; network download was not run. Local media import is independent |
| Welcome / onboarding / help | Yes | Yes | Yes: onboarding schema v2 | Unit tests for first run, skip, completion, replay, migration | Partial: current package CLI only | Welcome actions and help topics match current workspaces; no interactive clean-install GUI test |
| Render projects / profiles | Yes | Yes (GUI and CLI) | Yes: versioned atomic project/profile stores | Yes: path/schema/profile/queue tests; real project→queue→render test | Partial: CLI path smoke only | Project, profile, and immutable job snapshot are distinct |
| Job queue and render history | Yes | Yes (GUI and CLI) | Yes: versioned queue/history stores | Yes: state, cancellation, rerender, and real project queue-render tests | Partial: package `jobs list` only | Queue is active lifecycle; history records terminal attempts. This audit did not manually use the history GUI |
| Performance laboratory | Yes | Yes (GUI and CLI) | Yes: atomic versioned performance records | Real Larp render produced stage timings; synthetic/correctness tests cover comparison and queue-wait separation | Partial | One 13.379 s / 12-frame sample at 320×180, 6 fps is diagnostic only, not a benchmark or speed claim |
| Render diagnostics | Yes | Yes (GUI and CLI) | Yes: atomic versioned records | Real Larp diagnostics persisted with `LOCALAPPDATA` redirected inside the workspace | Partial | Default app-data writes were denied by this sandbox; redirected run succeeded |
| Compatibility analyzer | Yes | Yes (GUI and CLI) | Reports persist/export | Static and bounded dynamic-probe tests use shared Watch runtime | No | Analyzer does not compile GCC or prove rendered visual parity; unsupported/unknown results are distinguished |
| Regression laboratory | Yes | Yes (GUI and CLI) | Yes: versioned references/reports | Pixel/audio/semantic comparison logic, tolerances, hashes, and workload identity tests | No | No cross-version real-engine baseline comparison was run in this audit |
| Reproducible manifests | Yes | Yes (GUI and CLI) | Yes: atomic versioned manifest | Hash, redaction, missing/mismatch, compare, and reproduction-config tests | No | Reproduce is designed to submit a new job; no real historical manifest reproduction was run here |
| Notifications | Partial | Yes | Config is persisted; credentials are environment references | Mock backend/retry/redaction tests | No | Native notification attempt failed in this shell; Discord delivery was not network-tested. Render still completed |
| Current Windows release build | Yes | N/A | Build metadata | `cargo build --locked --release` passed | Current release binary built | `rc.exe` unavailable, so Windows executable version metadata was not embedded |
| Current portable release layout | Yes | CLI reached | Portable state is separate | Current-source portable ZIP was created and smoke-tested (see below) | Yes: CLI smoke only | Inno Setup/installer unavailable; packaged GUI, media import, and export not tested |

## Prompt-to-implementation traceability

The original prompt bodies are not a version-controlled specification set in this checkout. This practical trace uses the implementation reports and documentation that are present, then verifies current owners and behavior in source. It records families rather than claiming a line-by-line prompt archive.

| Requested capability family | Prompt lineage available in repository/context | Current implementation owner / user workflow | Audit status |
|---|---|---|---|
| Generic Sonolus rendering, Watch VM, particles, SFX/audio, GPU output | Early core prompts and #1–5; architecture/runtime reports | `formats.rs` → `watch_runtime.rs` / `runtime.rs` → `offline.rs` → `video_export.rs`; Quick Render and `render-video` | Implemented for supported operations; runtime gaps documented below |
| Gameplay UI, asset-driven presentation, profiles, attribution/watermark | #6–8 and later UI/profile work | `render_ui.rs`, profile/workspace controls; renderer configuration boundary | Implemented as data-driven presentation; actual client-oracle visual equivalence is unverified |
| Sono-GCC accelerator and performance | GCC feasibility/implementation/follow-ups; performance-lab prompt | `sono_gcc.rs`, `performance.rs`; config/CLI and Performance workspace | Optional backend with VM fallback; this audit ran VM for Larp and makes no speed claim |
| Configuration, profiles, jobs, notifications, cache, diagnostics | #8–21 series | `render.rs`, `config_profiles.rs`, `job_manager.rs`, `notifications.rs`, `render_history.rs`, cache/telemetry owners; GUI and CLI | Wired and persistence-tested; external OS/Discord delivery is not real-service tested |
| Compatibility, regression, reproducible manifests, maintainability | #16–22 series | `compatibility.rs`, `regression.rs`, `render_manifest.rs`, architecture docs | Production paths exist; exact cross-version real-output comparison/reproduction was not run in this audit |
| Gameplay presentation studio and asset use | #23 | `composition.rs`, Presentation Studio GUI, title/profile data | Distinct from the generic game renderer; current NLE FFmpeg integration tested |
| General NLE, standalone project, Shorts, timeline editing | #24, #30–32.8 | `composition.rs`, `timeline_playback.rs`, `gui.rs`; Presentation Studio | Ordinary-media project/export tested; interactive workflows, long playback, and exact Shorts acceptance remain unverified |
| Welcome/onboarding and discovery | #25, #33 | `onboarding.rs`, Welcome/Learn GUI | State transitions/migration tested; clean-install interactive UI was not exercised |
| Chart Studio / generic LevelData authoring | #26 | `chart_studio.rs`, canonical `LevelData`, project extension/export | Model/export/runtime pieces exist; human authoring round-trip is unverified |
| Release packaging and installation | #28 | `scripts/package-windows.ps1`, release/smoke scripts | Current-source portable CLI smoke passed; installer unavailable because `ISCC.exe` is absent |
| URL media acquisition | #29 | Media acquisition module and CLI/GUI actions | Workflow exists; pinned yt-dlp executable was absent, so acquisition was unavailable |
| Microphone capture | #32 | `audio_input.rs`, ordinary WAV media/timeline path | Production API/UI wiring and unit/recovery coverage exist; no real microphone was available |
| BPM / tempo detection | #34 | `tempo_analysis.rs`, Chart Studio analysis/apply | Synthetic MP3 decode and model tests pass; labeled real-song/user session not tested |
| Automatic subtitles | #35 | `subtitles.rs`, ordinary text clips and FFmpeg audio extraction | Interchange/editing and extraction tests pass; recognizer/model inference and caption export with real speech remain unverified |

## Additional audit findings

### Original render foundation and execution boundaries

Real supplied-engine parsing, initialization, Watch scheduling, frame output, and resource tests passed in the serial suite; a separate Larp render exercised direct engine + level + resources + music → H.264/AAC MP4 without a project or NLE. `bgmOffset` conversions have dedicated tests and the observed render used the VM interpreter. The Larp interval had scheduled loop events but no one-shot events; it therefore does not independently certify audible one-shot timing. GCC differential coverage passed in tests, but GCC was not selected for the Larp output.

`gpu_render.rs` owns CPU/WGPU gameplay drawing. Presentation Studio's composition render path is an FFmpeg filter/composition pipeline; it is not evidence of a general WGPU NLE compositor. Multiple NLE layers/effects/masks/transitions were exercised through the production FFmpeg integration fixture. A physical discrete GPU was not identified and no GPU utilization/VRAM measurement was taken.

### Reachability, dead code, duplicate concepts, and errors

- The current GUI and CLI entry points are enumerated above. Source scans found no production `todo!`/`unimplemented!`, constant-disabled major action, or obvious placeholder-success handler. These are static findings, not proof every GUI action was clicked.
- No substantial obsolete renderer/runtime was confidently identified and deleted. The compiler/check suite did not report dead-code warnings that motivated removal. Similar caches are subsystem-owned (e.g. native-code artifacts, thumbnails/proxies, transcription and BPM results); the central cache manager accounts for categories, while their distinct identities and lifetimes are deliberate rather than one universal cache schema.
- There are intentionally separate edit models for gameplay UI tracks and composition keyframes, and separately owned project/profile/job/history records. Tests and docs define their conversion edges; no accidental `Timeline2` or alternate Watch traversal was found in the audited paths. This is not a claim that every editor uses one universal undo command implementation: chart edits, composition edits, and render-job lifecycle have domain-specific state transitions.
- User paths, downloaded media arguments, profile/config values, and export errors have validation/subprocess boundaries in production modules. Tests cover credential/path redaction and manifest/project path safety. The audit did not fuzz every deserializer, exhaustively inspect every panic/unwrap, run a security scanner, or test every filesystem permission edge. No shell command concatenation with untrusted URL input was found in the media acquisition path inspected.

### Persistence, time, background work, dependencies, and package

The persistence-owner inventory above covers the product stores found in source: subsystem-local versioned schemas and atomic save/recovery code; cache entries are disposable. Tests cover migrations/corruption/future-version behavior for major stores. This is a source inventory, not a claim that power-loss durability was physically fault-injected on every platform.

Time remains domain-specific by design: Watch/media offset, rational gameplay frame time, NLE composition/source time, capture sample position, BPM media time, and subtitle clip-local/composition mapping each have named owners. Tests cover their core conversions; a single end-to-end random-seek scenario combining all of them was not driven. Long-running GUI background work uses bounded subsystem task/job paths with cancellation/stale-result checks where implemented; no worker stress/leak soak was run.

Dependency review was limited to the locked Cargo graph and the relevant FFmpeg/CPAL/Whisper/yt-dlp process or optional-tool boundaries. Cargo check/build resolved the locked graph successfully. This pass did not perform a full license/SBOM audit or broad dependency replacement. The current portable package smoke ran with restricted PATH and a foreign working directory. It did not validate packaged GUI, FFmpeg download on a clean machine, microphone, Whisper model, or installer behavior. `rc.exe` and Inno Setup are absent in this environment.

### Test outcomes by requested journey

| Journey | Status in this audit | Evidence / exact boundary |
|---|---|---|
| Direct engine + level + resources + music → MP4 | Real-engine tested | Larp VM/CPU render; FFprobe confirms 320×180 H.264/AAC, 12 frames, 2 seconds |
| Project save/reopen → queue → real render | Integration-tested with real fixture | Ignored `saved_project_can_queue_and_render_a_real_fixture` test passed |
| Ordinary MP4/audio/image → NLE preview/export | Real-media integration-tested | Ignored production FFmpeg test passed; diagnostic frames inspected, but editor GUI not manually operated |
| Chart author/edit/export/reimport/preview | Partial / not manually validated | Canonical model/export/runtime tests; full human round-trip not run |
| Vertical 1080×1920 composition/export | Real-media integration-tested | The NLE fixture exported/validated a vertical composition; the prescribed exact multi-instance gameplay/blur/caption workflow was not run |
| Microphone record/edit/export | Blocked from real-device validation | Unit/API/writer/recovery paths only; no device or OS permission exercise |
| BPM on real labeled song | Partial | Generated MP3 decode/alignment and model tests; no labeled corpus/user session |
| Speech → timed captions → burn-in | Partial / blocked | FFmpeg trimmed/rate-adjusted PCM extraction passed; no Whisper runtime/model or real speech |
| Full factory across acquisition, chart, gameplay, voiceover, subtitles, Short, manifest | Not end-to-end tested | Major pieces have independent evidence; no single integrated flagship project was run |

## Architecture and authoritative paths

This is one Cargo package, `renderer`. `src/main.rs` owns the Clap command tree and CLI orchestration; `src/gui.rs` owns the eframe front end. Both front ends call the library modules in `src/lib.rs`.

```text
GUI / CLI
  ├─ direct RenderConfig ───────────────┐
  ├─ RenderProject + profile resolution ├─> RenderJobManager (optional) ─> RenderConfig execution
  ├─ Chart Studio -> canonical LevelData┘
  └─ Presentation Studio -> CompositionProject -> FFmpeg preview/export

RenderConfig -> RenderConfig::render_frame / video_export
  -> FrameSession -> WatchRuntime -> WatchVm
      ├─ display list / particles / UI / audio requests
      └─ optional Sono-GCC scalar region with Watch VM fallback
  -> CPU or WGPU gameplay frame -> FFmpeg mux/validation -> result
  -> performance + diagnostic records; queue terminal outcome -> history
  -> manifest can verify inputs and submit a new job
```

The core owners found in code are: `formats.rs` for engine/resource/LevelData parsing; `runtime.rs` for `WatchVm`; `watch_runtime.rs` for entity lifecycle and host bindings; `offline.rs` for `WatchFrameStepper`/`FrameSession`; `sono_gcc.rs` for optional compiled scalar regions; `video_export.rs` for gameplay export; `gpu_render.rs` for CPU/WGPU gameplay rasterization; `composition.rs` and `timeline_playback.rs` for the independent NLE; `project.rs` for render projects; `job_manager.rs` for queue lifecycle; `render_history.rs`, `performance.rs`, `render_diagnostics.rs`, and `render_manifest.rs` for their respective persistent records.

### “One Watch Grandpa” verification

Production playback enters through `FrameSession`/`WatchFrameStepper`, which owns one `WatchRuntime`; that host evaluates callbacks through `WatchVm`. `video_export.rs` consumes prepared frames and does not re-evaluate gameplay. `sono_gcc.rs` replaces only eligible scalar VM regions; unsupported nodes and failed native execution return to the VM path. Compatibility probes and Chart Studio previews instantiate that same runtime, while `evaluate-node` is a single-node diagnostic, not another entity traversal. The full test suite passed the scheduler, real-engine initialization, GCC-vs-VM region, and export pipeline cases. This audit found no feature-specific parallel Watch runtime.

### Real Sonolus coverage limits

The compatibility analyzer and `docs/SONOLUS_V1_1_4_CAPABILITY_AUDIT.md` explicitly report incomplete support for **stack operations**, **replay stream provisioning**, and **curved Draw mesh rasterization**. The checked-in reference material does not provide enough authoritative opcode/geometry behavior to implement those safely by inference. They remain **partial/missing core compatibility**, not a green generic-engine claim. Static compatibility analysis also does not prove callback reachability or visible output, and it does not compile Sono-GCC.

## UI and CLI reachability audit

Current major GUI destinations are reachable from the Welcome/home or main workspace: Quick Render, Chart Studio, Presentation Studio, render-project open, Learn/Getting Started, gameplay UI/watermark settings, jobs, history, performance, diagnostics, regression, compatibility, cache, profiles, manifests, and URL acquisition. Presentation Studio contains current entry points for media import, tracks, source/program preview, transforms, effects, masks, transitions, subtitle import/export/generation, microphone meter/record, proxies, project save/reopen, and MP4 export. Chart Studio contains new/open, entity list/raw JSON, schema inspection, validation, Watch still preview, tempo analysis, save-to-render-project, and standalone gzip export.

The CLI `--help` inventory was executed. Current command families include direct `render-video`, `render-config`, project inspect/validate/render, engine/resource/level inspection, Watch evaluation/run, LevelData validation/export, compatibility, media URL inspect/download/tool-info, cache, jobs, history, manifests, performance, diagnostics, regression, and profiles. `media tool-info --json` returned `available: false` for the pinned yt-dlp binary. Chart authoring itself, NLE editing, microphone recording, BPM analysis, and subtitle generation are GUI workflows, not CLI commands. No constant-disabled button or TODO/unimplemented macro was found by the source scan. This was static reachability inspection plus build/test evidence, not a mouse-driven GUI audit.

The classic renderer's **Program Preview** is intentionally a still-frame monitor; it is not presented as Presentation Studio playback. Presentation Studio itself has a separate continuous timeline preview. The distinction is explicit in the current UI/docs.

## Project, configuration, and persistence ownership

| Data | Owner / version | Persistence behavior and audit result |
|---|---|---|
| Renderer configuration | `render.rs`, schema 1 | Shared GUI/CLI/profile `RenderConfig`; atomic save; legacy flat input compatibility and future-version rejection are tested |
| Render project / recents | `project.rs`, schema 1 | Atomic `.sonoproject`; relative paths, unknown extension fields, required profile identity, moved project resolution, and future-version rejection are tested |
| Presentation composition | `composition.rs`, schema 21 | Atomic `.sonocomp`; migrations 1–20, unknown-field preservation, future-version rejection, relative media paths, and recovery checkpoints are tested |
| Chart authoring schema/snapshot | `chart_studio.rs`, schema 1 | Engine content-hash-bound declarative schema; canonical LevelData snapshot lives in render-project extension; gzip export is deterministic and atomic |
| Gameplay UI / watermark profile | `render_ui.rs` / `watermark.rs`, schema 1 | User-owned profile JSON; asset paths/variants validated; UI and watermark master-off behavior have tests |
| Job queue | `job_manager.rs`, store version 1 | Atomic shared store; queue transitions, recovery, cancellation, merge, output reservation, and history projection tests pass |
| Render history / retention | `render_history.rs`, schema 1 | One JSON record per attempt; malformed records are isolated; retention/pinning/failure behavior tested |
| Performance / diagnostics | `performance.rs` / `render_diagnostics.rs`, schema 1 | Atomic record stores with retention; path and credential redaction tests pass |
| Manifest / regression reference | `render_manifest.rs` / `regression.rs`, schema 1 | Atomic export, content identity, unknown-field preservation or fail-closed reproduction behavior, and comparison checks are covered |
| Profiles / notifications / onboarding | `config_profiles.rs` / `notifications.rs` / `onboarding.rs`, schema 1 / 1 / 2 | Atomic versioned catalogs/settings; profile export and history snapshots redact credentials; onboarding skip/complete/replay migration tested |
| Audio preferences / partial takes | `audio_input.rs`, schema 1 | Stable CPAL device ID preference; staged WAV/checkpoint and partial recovery tests; actual device was unavailable to validate |
| BPM / transcription caches | `tempo_analysis.rs` / `subtitles.rs`, schema 1 | Content/model/settings-keyed disposable caches, bounded cleanup; accepted caption text is project data and survives cache deletion |

There is no single database or universal persistent document: each schema has a subsystem owner. Subsystem-local decoded SFX, thumbnail, transcription, tempo, and Sono-GCC caches remain separate because their entries/lifetimes differ; the cache inventory/clear/prune workflow aggregates managed disk categories. A fully mechanical audit of every framework-managed GUI preference and platform filesystem edge was not performed.

The primary media/time conversions are explicit: gameplay export uses frame/rational time and `bgmOffset`; the project tests verify `media time = Watch time + bgmOffset`. NLE composition time is separate from source-media time, with clip In/Out and source-time mapping owned by `composition.rs`; microphone capture stores start position plus sample frames; subtitle extraction maps source range/rate into composition time; BPM analysis uses decoded media time. Tests cover these conversion pairs. Direct random seeking of all combinations of gameplay event state, arbitrary NLE effects, audio hardware, and captions was not manually exercised.

## Findings by risk class

### Required / working baseline

- The direct Sonolus render path does not require projects, Chart Studio, Presentation Studio, profiles, or optional subsystems.
- The ordinary-media NLE path does not load EngineData, LevelData, Watch, or Sono-GCC.
- Current `CompositionProject` has real import/edit/filter/mix/export production paths and project persistence; the actual FFmpeg acceptance test passed.
- Queue/history/profile/manifest/diagnostic/performance and regression code are wired to front ends; current CLI parsing and subsystem tests passed.

### Partial implementation needing future completion

- Complete Sonolus v1.1.4 stack operation semantics, replay-stream provisioning, and curved Draw mesh rasterization. Existing available references are insufficiently authoritative for a safe implementation; those are explicit compatibility gaps.
- A universal typed property/keyframe graph shared across gameplay UI and NLE is not present. Each existing domain has its own project model and editor, with shared easing helpers. Consolidating them would be an architectural change, not a hidden completed feature.
- NLE lacks dedicated roll/slide tools, custom cubic-Bezier easing/curve handles, and freeform masks. The baseline cut/trim/move/slip/ripple/insert/overwrite/group/linked-A/V operations do exist.
- Presentation Studio has no live Watch-backed source. It edits a previously rendered gameplay/video file as normal media.
- Chart Studio is generic LevelData entity authoring, not an engine-specific spatial chart editor; it does not infer entity semantics or provide placement/graph tools.
- Notifications have production adapters/configuration but no successful OS notification or Discord service interaction in this environment.

### Blocked validation (not converted to success claims)

- No mouse/keyboard automation or interactive desktop session was available. Welcome, NLE, Chart Studio, history, microphone, and actual export GUI workflows were not manually driven.
- No microphone device/permission-change/hot-plug test was available. CPAL unit tests and writer/recovery tests are not real-device evidence.
- No whisper.cpp runtime/model was installed, so real speech, silence/music hallucination, multilingual accuracy, captions-on-video export, and packaged subtitle inference remain unverified. FFmpeg extraction of a trimmed/rate-adjusted sample did pass.
- No yt-dlp executable/network acquisition was performed. Local media remains independent.
- Inno Setup 6 (`ISCC.exe`) is absent; no installer was produced. A current portable ZIP was packaged and CLI-smoke-tested, not its GUI or full media workflow.
- The physical GPU adapter was not identified. WGPU tests passed but do not establish discrete-GPU behavior.
- The current FFmpeg binary pair is discoverable and its version banners match. The test run used the shared add-on pair; a clean offline install or first-run download was not tested.

### Performance / resource observations

The 2-second Larp probe rendered 12 frames at 320×180 / 6 fps in 13.379 seconds on this machine. Reported stages were ~7.412 s preparation, ~5.162 s rendering, ~0.612 s audio mixing, ~0.134 s audio encoding, and ~0.059 s finalizing. The output rate was 3 fps. This is a single small-resolution render with a high first-frame/setup cost; it is **not** an apples-to-apples benchmark or a performance claim.

The first CLI run could not write performance/diagnostic records under the sandbox-denied default `LOCALAPPDATA`; the render still succeeded. Re-running with `LOCALAPPDATA` redirected to an audit-local directory persisted a 2.8 KB performance record and 9.0 KB diagnostics record. Notification delivery also failed in the shell without failing the render. No long-run memory, CPU/GPU, preview-drop, device-leak, or network-load profiling was done.

## Concrete end-to-end evidence

### Build and tests

- `cargo fmt --all -- --check` — passed (Cargo emitted `could not canonicalize path C:\Users\Admin`).
- `cargo check --locked --all-targets` — passed; `rc.exe unavailable` means Windows executable version metadata was not embedded.
- `cargo test --locked -- --test-threads=1` — passed: **387 lib + 27 CLI + 3 `arctan2` + 101 `m0` + 9 Watch scheduler = 527 passed, 0 failed, 7 ignored**. The ignored cases include optional FFmpeg integration and private forensic fixtures; three FFmpeg/queue integration tests were separately run below.
- `cargo build --locked --release` — passed in 2m09s, with the same `rc.exe` warning.
- Clippy was not rerun during #36. Prior #35 report recorded strict Clippy failure from repository-wide existing diagnostics; this report does not claim it is clean.

### Real input / output

- **Larp 64x render:** direct current debug executable with `TestingSuite/Next Sekai Engine/levels/larp 64x/{Next RUSH.zip, ProSeka Faithful 0.8.4.scp, larp 64x.json.gz, larp 64x.mp3}`, VM/CPU, 0–2 s, 320×180, 6 fps. FFprobe verified H.264 + AAC, 12 frames, 2.000 s. RGB sequence SHA-1: `f42655066ad2970d3083d4ab132f1f18708b15c6`; SFX event sequence SHA-1: `065623858bbffa464f9c2da960afbeefd8d4586b`. `artifacts/feature-existence-audit/larp-2s-320x180.mp4` SHA-256: `A47A176E90D8BE44B8E804BFBD3A7BC29F6047B5265C8A599828437C949DA785`.
- **Frame inspection:** `artifacts/feature-existence-audit/larp-1_5s.png` SHA-256 `89C45B58788F2665C29191EE4944A900AEE0700A57EAB9C811B84B826AF64C65`. I inspected it at 320×180: the playfield, notes, background, and attribution are visible; no obvious full-frame clipping/missing-resource failure. This is not high-resolution visual certification.
- **Production NLE:** `cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture` — passed in 79.75 s. It generated and FFprobe-validated ordinary-media previews/exports, including landscape and vertical compositions, range/nested output, real transitions/effects/masks, and proxy behavior. I inspected the generated landscape, vertical, transition-midpoint, and proxy frames. The fixtures intentionally use vivid diagnostic patterns and overlapping layers; they are not polished creator edits. No unexpected missing source appeared. The existing test's vertical frame shows the intentionally layered test layout, not a production Shorts design review.
- **Continuous decode pipe:** `cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture` — passed (0.58 s).
- **Subtitle audio preparation:** `cargo test --locked --lib subtitles::tests::real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm -- --ignored --nocapture` — passed (0.42 s). This validates FFmpeg extraction only, not Whisper inference.
- **Project → queue → render:** `cargo test --locked --lib project::tests::saved_project_can_queue_and_render_a_real_fixture -- --ignored --nocapture` — passed (7.48 s) using supplied Next Sekai engine, resources, level, and audio; the project was saved/reopened, queued, rendered, and produced a nonempty MP4.
- **Current packaged portable ZIP:** `scripts/package-windows.ps1 -Channel development -SkipBuild` created a ZIP from the release build. Its SHA-256 was `4A65A5AA0BA1EC90EED7AB318C4CFB07B85364C14FDED6CE2354860948222623`. `scripts/smoke-test-windows-package.ps1 -DistributionDirectory target\distribution -PortableOnly` passed extraction into Unicode/space paths, launch from a foreign working directory with restricted `PATH`, and packaged `--version`, `--help`, `jobs list`, and `media tool-info`. The package/staging directories were restored to their pre-test contents afterward; the tested ZIP was not retained. The smoke did not open the GUI, import media, or render from the package.

## Required acceptance workflows not completed end-to-end

| Workflow | What current evidence reaches | Missing certification step |
|---|---|---|
| Classic render | Real Larp/H264/AAC and real project-queue render | Repeat as a fresh packaged GUI job and verify saved history/notification in normal user data |
| Chart Studio | Canonical LevelData model/export/runtime tests and actual engine fixtures | Human create/edit/export/reopen/preview/normal MP4 interaction, across two materially different engines |
| Standalone NLE | Production compositor/filter graph FFmpeg integration, project persistence tests | User-operated GUI sequence from new project through edit/export; hardware audio output and long playback |
| Shorts | 1080×1920 real-media export from landscape input | Exact same-gameplay-source multi-instance, blur/caption/audio acceptance and GUI workflow |
| Microphone | CPAL setup, WAV writer, recovery, and ordinary clip model tests | Real device, capture, permission/hot-plug, drift, and final export |
| BPM | Synthetic MP3 decode/playback/time alignment and apply/undo/persist tests | Labeled real songs and interactive chart session |
| Subtitles | Unicode SRT/VTT plus real source extraction and clip mapping | Local model installation, real speech inference/review/style/export and package model provisioning |
| Full factory | Core project/render/NLE pieces pass independently | Network media acquisition, genuine chart authorship, real microphone, Whisper captions, and one packaged final-video workflow |

## Priority remainder

| Class | Remaining item | Action / reason |
|---|---|---|
| A — required and feasible now | No newly demonstrated baseline defect in the ordinary local render/NLE production paths | No speculative code rewrite was made; the real output and production FFmpeg test passed |
| B — required but environment-blocked | Real mic, OS permission/hot-plug, Whisper speech, URL acquisition, interactive desktop flow, discrete GPU identification, fresh installer | Provide hardware/model/network/ISCC/GUI automation and rerun the exact acceptance steps; absence of evidence is not a pass |
| C — partial and needs completion | Sonolus stack ops, replay-stream handling, curved Draw geometry; exact short-form multi-instance workflow; shared typed-property architecture | Core behavior needs authoritative semantics/fixture evidence before implementation. Do not guess a runtime ABI or geometry |
| D — intentionally limited | Live Watch source inside NLE, freeform masks, cubic-Bezier curve handles, roll/slide tools, speaker monitoring, word-level karaoke | These are documented limits or advanced workflows; decide product scope explicitly before claiming them |
| E — future/non-NLE scope | Professional DAW, VST/MIDI, collaboration/cloud, AI transcription service, advanced node compositor | Not required for this renderer/editor acceptance pass |

## Fixes made and outstanding debt

1. Added this evidence-based matrix/report and linked it from the README documentation index.
2. Added a dated correction to the historical #35 report: this audit found an available validated FFmpeg/FFprobe pair and ran its previously ignored integration paths. The old statement remains the earlier run's status, not current #36 evidence.
3. No production code was changed. The UI/source audit found no placeholder button that always returns success, no `todo!`/`unimplemented!` in production paths, and no constant-disabled major control. Static scans do not replace interactive UI testing.
4. Technical debt that remains concrete: `src/gui.rs` remains the broad integration surface; gameplay UI and NLE animation persist different domain models; compatibility is intentionally partial for the operations above; external device/network/model/package boundaries lack real-device tests.

## Reproduction commands

From the repository root on Windows with the app-discoverable FFmpeg/FFprobe pair:

```powershell
cargo fmt --all -- --check
cargo check --locked --all-targets
cargo test --locked -- --test-threads=1
cargo build --locked --release
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
cargo test --locked --lib timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes -- --ignored --nocapture
cargo test --locked --lib subtitles::tests::real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm -- --ignored --nocapture
cargo test --locked --lib project::tests::saved_project_can_queue_and_render_a_real_fixture -- --ignored --nocapture
```

`artifacts/feature-existence-audit/larp-2s-320x180.mp4` is a short generated real-engine output; `larp-1_5s.png` is its extracted frame. Other small diagnostic frames are in `artifacts/nle-validation/`. `dependencies/ffmpeg/` and `TestingSuite/` are ignored local fixtures/tools, not packaged source dependencies.

## Final conclusion

The repository does not merely contain names for most of these systems: the current source wires them to real GUI/CLI routes, versioned persistence, and production render paths, and the build/test/real-media/package evidence above confirms multiple boundaries. It is not accurate to claim that every advertised advanced feature is complete or that the product has passed a human-operated, real-device, clean-installed end-to-end run. The next work should be driven by the categorized remainder and authoritative engine/device evidence, not another blanket feature list.
