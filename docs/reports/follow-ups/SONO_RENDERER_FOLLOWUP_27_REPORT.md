# Sono-Renderer Follow-up #27 — architecture consolidation report

## Scope and repository state

This was a behavior-preserving audit and cleanup. No product feature was
added. The workspace already contained extensive uncommitted implementation,
documentation, fixture, and report changes from prior follow-ups. I preserved
that state and did not reset or clean unrelated work. The audit describes the
repository at commit `fe8dc1e` plus its working-tree changes.

The repository is one Rust Cargo package with a library and a CLI/desktop
binary. The GUI, CLI and renderer share the same configuration, project,
queue, Watch and export implementations. The prior architecture guide was
useful, but predated Chart Studio. This pass adds its current boundary, time
ownership, workspace state, and a standalone invariants document.

## Code changes

- Added shared Watch/media clock conversion helpers in `export_timeline.rs`
  and routed export timeline sampling, BGM bounds, audio-window setup, and
  Chart Studio media-time seeking through them. The behavior remains
  `media = Watch + bgmOffset`; inverse conversion is `Watch = media -
  bgmOffset`.
- Grouped Chart Studio's document, selection, draft, schema, preview and worker
  state into `ChartStudioView`. This makes the workspace boundary explicit
  without moving or duplicating its editing logic.
- Renamed the internal atomic counter call to `AtomicI64::try_update` to remove
  the toolchain deprecation warning. Kept the progress pulse helper only in
  test builds because its only caller is a test-only event-pass harness.
- Added `docs/ARCHITECTURAL_INVARIANTS.md`, updated `docs/ARCHITECTURE.md`, and
  linked both this report and the invariant list from the README.
- No dependency, persistent schema, CLI contract, render configuration
  meaning, or output behavior was intentionally changed.

## Architecture and ownership audit

The actual dependency direction remains:

```text
CLI / Desktop
    -> RenderProject + ConfigProfile resolution
    -> RenderConfig
    -> optional RenderJobManager snapshot
    -> render.rs / video_export.rs
    -> FrameSession / WatchRuntime / WatchVm
    -> PreparedFrame -> CPU or WGPU -> FFmpeg / output validation
    -> performance + diagnostics -> history; manifest can submit a new job

Chart Studio -> ChartDocument -> formats::LevelData
      |                         |              |
      +-> RenderProject extension              +-> normal Watch/render path
      +-> standalone LevelData gzip export
```

The subsystem map was checked against `src/lib.rs`, `src/main.rs`, the model
definitions and production call paths:

| Boundary | Current owner | Audit result |
|---|---|---|
| CLI command tree | `src/main.rs` | Commands converge on library loaders, configuration and render code. |
| Desktop workspaces | `src/gui.rs` | One eframe frontend. Chart state is grouped; the overall `Desktop` type remains large. |
| Sonolus package and LevelData | `src/formats.rs` | One canonical `EnginePackage`, `LevelData` and `LevelEntity` model. |
| Watch graph / operation evaluation | `src/watch.rs`, `src/runtime.rs` | One graph model and one `WatchVm` operation evaluator. |
| Watch lifecycle / callbacks | `src/watch_runtime.rs` | One `WatchRuntime` host; owns lifecycle, scheduling, resource binding and callback execution. |
| Frame progression / snapshots | `src/offline.rs` | `WatchFrameStepper`, `FrameSession`, `PreparedFrame`; downstream consumers use owned frame data. |
| High-level render config / entry | `src/render.rs` | Shared CLI, GUI, preview and video configuration boundary. |
| Video/export coordination | `src/video_export.rs` | Frame pipeline, audio preparation, FFmpeg handoff, validation and final publication. |
| Raster/composition | `src/gpu_render.rs` | CPU and WGPU paths; existing parity tests pass. |
| Gameplay UI / watermark | `src/render_ui.rs`, `src/watermark.rs` | Configuration-driven visual consumers of runtime state. |
| Chart authoring | `src/chart_studio.rs` | Edits canonical LevelData; exports standard LevelData. |
| Project / profile | `src/project.rs`, `src/config_profiles.rs` | Project is subject/input state; profile is reusable RenderConfig preferences. |
| Queue / history / manifest | `src/job_manager.rs`, `src/render_history.rs`, `src/render_manifest.rs` | Snapshot execution, terminal attempt records and reproduction descriptions remain distinct. |
| Telemetry / diagnostics / compatibility | `src/performance.rs`, `src/render_diagnostics.rs`, `src/compatibility.rs` | Performance is authoritative; diagnostics reference it; compatibility invokes the Watch host. |
| Cache / identity | `src/cache_manager.rs`, `src/content_hash.rs`, `src/sono_gcc.rs` | Manager handles generated cache entries; exact compiler identity remains compiler-owned. |
| Notifications / platform work | `src/notifications.rs`, platform branches in owners | Best-effort bounded delivery; failures do not determine render success. |

The current repository has one project envelope (`RenderProject`). Chart
Studio stores a versioned canonical LevelData snapshot under a project
extension key. No separate chart/video project was found. Presentation
settings can be part of `RenderConfig`, but there is no persisted multi-track
composition model in this checkout.

The dependency inspection found producer/consumer links, not an evident
semantic cycle: diagnostics consume performance records; regression calls the
ordinary renderer; queue code projects terminal outcomes to history; project
resolution submits a resolved config. `job_config.rs` and `store_paths.rs`
already isolate scheduler settings and default store paths. No automated
module-cycle analyzer was available, so the cycle result is a source-level
inspection rather than a formal graph proof.

## Required 60-point findings

Status words distinguish inspected code from exercised behavior. “Tested”
means a test or command actually ran in this pass; “deferred” means the
requested workflow does not exist and was not fabricated for this cleanup.

1. **Repository state — verified.** One package, a library, a CLI/GUI binary,
   supplied real-engine fixtures and many prior uncommitted changes were
   present. Existing work was preserved.
2. **Subsystem inventory — inspected.** The ownership table above reflects
   the current source modules and types, not previous prompt completion claims.
3. **Incomplete systems — inspected/deferred.** Chart authoring has no spatial
   canvas or universal beat grid. Program Preview renders still frames; a
   nonlinear multi-track editor is absent. Those were not added.
4. **Architecture before — inspected.** The core render path was already
   shared, but architecture docs did not yet map Chart Studio and its project
   extension.
5. **Architecture after — documented.** The architecture guide now covers
   Chart Studio, Program Preview, canonical LevelData and shared time mapping.
6. **Cycles — source inspected.** No duplicate Watch/runtime or obvious
   subsystem cycle was found. There is no formal dependency graph check.
7. **Watch runtime — verified/tested.** `WatchVm` evaluates Watch operations;
   `WatchRuntime` owns callbacks/lifecycle. Export, compatibility, preview and
   regression call the same host/evaluator path. The full scheduler and real
   fixture tests passed.
8. **Sono-GCC — differential-tested/real-engine tested.** Native regions
   execute eligible scalar operations; Watch VM handles cuts and fallback.
   Real Next RUSH output matched the VM frame exactly. This is not a claim
   that all workloads are faster under GCC.
9. **LevelData — verified.** `formats::LevelData` / `LevelEntity` are the
   shared model used by import, Chart Studio and export; unknown JSON fields
   are preserved.
10. **LevelData export — integration-tested.** The CLI exported 1,131 real
    entities to standalone gzip, validation reported zero errors/warnings,
    and the normal Watch run produced identical output hashes.
11. **Time model — code-cleaned/unit-tested.** One pair of named conversion
    helpers now owns the LevelData offset relation. Positive and negative
    offset inverse tests pass.
12. **Seeking — unit-tested/partially inspected.** Watch backward-seek and
    runtime state tests pass; stale preview results are revision-checked.
    Rapid interactive GUI seeks, nested compositions and every event boundary
    were not manually exercised.
13. **Project model — verified/tested.** `RenderProject` is the only project
    envelope. Chart Studio's versioned LevelData extension survives the
    existing project snapshot test. It is not a multi-track editor project.
14. **Profile/job/history/manifest — integration-tested.** CLI and library
    tests cover captured profile config, queue lifecycle, history rerender and
    manifest reproduction as separate concepts. This pass did not redesign
    them.
15. **Commands/undo — inspected.** Chart entity operations use one bounded
    ChartDocument operation log. There is no second timeline undo stack
    because there is no nonlinear timeline editor. A cross-workspace command
    framework would add no current shared operation semantics.
16. **Assets — inspected.** Sonolus engine/SCP assets, presentation asset-pack
    entries, external media paths and derived caches have distinct owners.
    No universal asset database was found or introduced.
17. **Content identity — inspected/tested.** SHA-256 helpers support portable
    content identities; SHA-1 remains in Sonolus payload conventions and
    legacy RGB/audio/compiler evidence. They are purpose-labeled but not
    wrapped in one strongly typed identity type.
18. **Cache — integration-tested in isolation.** A 72.4 MiB generated GCC
    cache in the ignored validation directory was cleared with the cache CLI;
    it went to zero, then GCC rebuilt and reproduced the same frame hash.
    User cache and source/project data were not touched.
19. **Background tasks — inspected/tested.** Queue workers, the bounded Watch
    callback pool, notification worker, compiler gates and GUI-owned workers
    have distinct owners. Render cancellation and stale preview suppression
    exist. There is no universal task executor, intentionally.
20. **GPU ownership — inspected/tested.** WGPU and CPU raster paths remain
    behind `gpu_render`; frame-pipeline and CPU/WGPU parity tests passed. No
    GPU ownership rewrite or allocation optimization was attempted.
21. **Media pipeline — integration-tested.** A direct three-frame render
    exercised Watch, CPU rendering, SFX collection, BGM audio, FFmpeg encode,
    mux and FFprobe validation. Multi-track source-video editing is absent.
22. **Audio — integration-tested/limited.** Audio offset tests and the actual
    MP4 AAC output passed. Long-duration drift and multi-clip audio automation
    were not tested because no NLE timeline exists.
23. **Presentation state — unit-tested.** Existing tests preserve the separate
    event-derived score/combo/judgement and authored LIFE behavior. This pass
    did not change gameplay presentation semantics.
24. **Property/animation — unit-tested/limited.** Existing typed profile and
    interpolation tests pass. There is no single general-purpose property
    editor shared with a nonlinear composition system.
25. **Timeline — documented.** Export time, schema-declared Chart Studio
    timing, and gameplay Watch beat conversion have separate meanings. No
    universal editor timeline or BPM grid was inferred.
26. **Chart Studio — integration-tested.** Generic edit/undo/export tests
    passed; the real level export/validate/Watch chain passed. A semantic edit
    of a real chart inside the GUI was not performed.
27. **Presentation Studio — inspected/deferred.** Gameplay UI profiles and
    Program Preview exist. Program Preview is a still-frame monitor, not a
    multi-track nonlinear video editor.
28. **Cross-workspace flow — partially verified.** Chart data can be saved in
    the render project, exported independently, and rendered through the
    common path. There is no live chart-to-composition handoff because the
    composition editor does not exist.
29. **Onboarding — build/source inspected.** Existing first-run onboarding
    and help remain additive. No onboarding redesign or desktop interaction
    was performed.
30. **Errors — inspected.** Boundary functions retain `anyhow` context;
    states such as job failure, history status, compatibility and diagnostics
    use typed enums. A repository-wide error-type migration was not justified.
31. **Persistence — inspected/tested.** Project, profile, job, history,
    performance, diagnostics, manifest, cache and Chart Studio payloads have
    owner-specific versioning and atomic/recovery tests where applicable.
32. **Filesystem layout — inspected.** Shared store path helpers own queue /
    history defaults; other stores use platform state paths. The real MP4
    succeeded while optional performance/diagnostic persistence reported
    access denied under this environment's `%LOCALAPPDATA%`.
33. **Dependencies — inspected/build-checked.** `cargo tree -d` shows version
    duplication concentrated in transitive GUI/WGPU/platform stacks. No direct
    dependency was removed. `cargo machete` is not installed, so automated
    unused-dependency analysis was unavailable.
34. **Unsafe/FFI — manually inspected.** Unsafe boundaries are limited to
    Sono-GCC native DLL/ABI loading and invocation, GUI console handling, and
    job-manager process-liveness checks. GPU and FFmpeg use safe Rust APIs.
    Linux/macOS were not built in this environment.
35. **Dead code — cleanup/build-checked.** Replaced deprecated internal
    `fetch_update`; gated `ProgressTracker::pulse` to test builds because its
    only caller is test-only. No broad dead-code deletion was done. Clippy
    still reports unrelated warnings.
36. **God objects — partially cleaned.** Chart Studio state is now a cohesive
    `ChartStudioView`. The much larger `Desktop`, `main.rs`, `runtime.rs` and
    `video_export.rs` were not split mechanically; each has real cross-cutting
    orchestration that needs a separately scoped boundary plan.
37. **Copies/allocations — inspected, not optimized.** `PreparedFrame` and
    bounded channel ownership are intentional. No allocation change was made
    without a measured hot-path issue.
38. **Locks — source inspected.** Queue persistence and state synchronization
    remain the broadest lock scope; rendering/compiler/network work is outside
    that lock. No contention benchmark or lock-order stress test was added.
39. **Performance baseline — measured, limited.** Performance Lab ran the
    real Baumkuchen workload at 320x180, 12 FPS, 0.25s output, one warm-up and
    three measured runs per backend. This small workload is a repeatable
    datapoint, not a general renderer benchmark.
40. **VM/GCC comparison — measured.** VM median was 2,216.98 ms; warm GCC
    median was 2,139.98 ms. The 77.00 ms / 3.47% difference is classified
    `within_noise` by the lab. GCC's initial state was `miss_compiled`; the
    measured runs were `process_hit`. Three samples do not support p95.
41. **Real-engine matrix — tested.** Next RUSH/Baumkuchen and Horizon/Dreamer
    ran through the real Watch host. Next RUSH also ran native GCC. This is a
    two-engine point-frame matrix, not exhaustive engine/resource/audio
    coverage.
42. **Quick Render/direct workflow — integration-tested.** Ordinary
    `render-video` produced a valid H.264/AAC MP4 without project/history/
    profile/GUI inputs. FFprobe confirmed 320x180, 12 FPS, 3 frames and 0.25s.
43. **Chart Studio E2E — integration-tested/partial.** Real LevelData CLI
    export -> structural/engine validation -> ordinary Watch -> identical PPM
    succeeded. GUI semantic editing was not part of this chain.
44. **Presentation E2E — not available.** There is no multi-track editor
    project/export workflow to exercise. Existing gameplay UI compositor tests
    are not misrepresented as a nonlinear edit/export test.
45. **Full authoring-to-NLE pipeline — deferred.** The authoring-to-LevelData-
    to-Watch-to-MP4 segment works. NLE composition/audio-track stages do not
    exist, so the complete requested pipeline cannot be claimed.
46. **Save/reopen — unit-tested/partial.** Existing project tests reopen the
    Chart Studio LevelData extension. Closing/reopening the native GUI and
    continuing an edit was unavailable.
47. **Cache deletion — integration-tested.** Only the isolated generated
    compiler cache was cleared; project/input data remained outside the cache
    store. The rebuilt GCC output matched the original VM frame.
48. **Missing media/resource — unit-tested/partial.** Project preflight,
    compatibility, manifest repair and media-validation cases pass. An
    interactive missing-file relink flow was not exercised.
49. **Crash recovery — unit-tested/partial.** Atomic persistence, interrupted
    queue recovery and export failure cleanup are covered. Chart autosave
    recovery does not exist and no GUI crash simulation was performed.
50. **Determinism — tested.** Pre/post-cleanup display-list JSON and PPM
    SHA-256s matched for both fixtures. VM and GCC output hashes matched for
    the same Next RUSH frame. Gzip LevelData export hash remained
    `ef36debb4bb9fe6c3c0ac8b51336a1ce0aad618b6f0d8494f9805aac1290de78`.
51. **Test quality — inspected/tested.** Existing test names identify semantic
    invariants; real fixture, differential, round-trip, and failure-path tests
    ran. Test architecture was not reorganized in this behavior-preserving
    pass.
52. **Flaky tests — limited audit.** The full suite ran once before and once
    after; all passed both times. This was not a repeated-run statistical
    flakiness study.
53. **GUI — build-checked only.** `cargo check --all-targets` and the release
    build cover GUI compilation. Interactive navigation, dialogs, drag/drop
    and editing were not manually tested.
54. **CLI — integration-tested.** Help, LevelData export/validation, Watch,
    render-video, performance benchmark, cache clear, and existing CLI parser
    tests passed. The simple direct-input path remains available.
55. **Documentation — updated.** Architecture now includes Chart Studio,
    time mapping, project extension and actual Program Preview limits. README
    links a concise invariants page and this audit.
56. **Invariants — documented.** VM/GCC parity, single Watch lifecycle,
    canonical LevelData, immutable job snapshots, presentation isolation,
    time mapping, disposable cache, secret exclusion, atomic output and
    optional services are in `docs/ARCHITECTURAL_INVARIANTS.md`.
57. **Backward compatibility — tested, unchanged by design.** Existing CLI
    parser, LevelData loader/export, project and profile tests pass. This pass
    changes no serialized schema or public configuration field.
58. **Known limits — explicit.** No nonlinear editor, universal chart
    timeline, interactive GUI test, Linux/macOS build, long-duration sync test,
    or exhaustive real-engine matrix was completed.
59. **Future work — not implemented.** Larger workspace extraction, a true
    composition editor, schema-authored chart timeline tools, cross-platform
    testing and typed content identity remain future work, not hidden in this
    cleanup.
60. **Remaining debt — documented.** `gui.rs` remains approximately 380 KB,
    `main.rs` 214 KB, `runtime.rs` 169 KB, `video_export.rs` 147 KB, and
    several service modules are large. A safe next pass should extract one
    well-defined workspace/service boundary at a time with the same real
    fixture evidence.

## Validation record

### Baseline before edits

- `cargo fmt --all -- --check` — passed (Cargo emitted an environment path
  canonicalization warning for `C:\Users\Admin`).
- `cargo check --all-targets` — passed, with the then-existing deprecated
  atomic API and test-only progress helper warnings.
- `cargo test` — 281 library, 25 CLI, 3 `arctan2`, 101 `m0`, and 9 scheduler
  tests passed; 4 ignored. The library suite took 252.19 seconds; the `m0`
  fixture target took 87.75 seconds.
- `cargo clippy --all-targets` — completed with the repository's existing
  style warnings.
- `cargo build --release` — passed.
- Pre-edit release real-frame references were saved under
  `target/followup27-validation/baseline/`.

### After edits

- `cargo fmt --all` — passed.
- `cargo check --all-targets` — passed without Rust warnings.
- `cargo test` — 282 library, 25 CLI, 3 `arctan2`, 101 `m0`, and 9 scheduler
  tests passed; 4 ignored. The new test covers the shared media-time inverse
  pair. No hard timing assertions were added.
- `cargo clippy --all-targets` — completed successfully. It still reports
  broad existing lint warnings (88 library, 142 library-test warnings, with
  duplicate warnings across targets); the deprecated atomic and production
  dead-code warnings were removed.
- `cargo build --release` — passed.
- `git diff --check` — passed; Git only printed its existing LF/CRLF working
  copy notices.
- CLI render output: `target/followup27-validation/post/quick-render.mp4`,
  SHA-256 `36807CE70A68CB7C6870F4D6362F39259EAFC539711B9B0FE48CD6EF17789B2F`.
  FFprobe confirmed H.264, AAC stereo 44.1 kHz, 320x180, 12 FPS, 3 frames,
  0.25 seconds. One extracted PNG frame was opened and visually inspected;
  it showed rendered gameplay rather than a blank/corrupt frame. This is not
  an interactive GUI inspection.
- Cross-version real output hashes (pre-edit release vs final release):

| Fixture and time | Runtime summary | PPM SHA-256 (both versions) | Draw JSON SHA-256 (both versions) |
|---|---|---|---|
| Next RUSH / Baumkuchen, 6.336207s | 1,131 input entities; 3,602 runtime entities; 8,319 callbacks; 90 sprites | `BC2BBA49BD98553FF751CA9E9579170E92D303C1E65AA4BAEDE0F0534B97E112` | `FF489761328F3CF9E2B05B7B2366BAB8442AEAFBB9EEE2E0CC9C0018848F798A` |
| Horizon / Dreamer, 14.45s | 1,583 input entities; 2,229 runtime entities; 6,010 callbacks; 40 sprites | `616A9E308F784D1E43418BEC87C2AB7C2DFCBD25F0DCBE4F668CF0FB090F27AB` | `BCCAE6AAE2986DC93D3AB4815C6F68D21B01765818AA2F704BCEE589FB237E77` |

The Next RUSH Sono-GCC frame also matched VM's PPM and display-list hashes;
597,600 native regions executed in the frame. After clearing the isolated
72.4 MiB GCC cache, it recompiled and produced the same PPM hash. The cache
clear did not target the normal user cache.

The Performance Lab used the same Baumkuchen inputs for three measured runs
per backend after one warm-up. It recorded GCC's initial `miss_compiled`
observation (59.03 s compile) followed by `process_hit` measured runs. VM
median was 2,216.98 ms and GCC median was 2,139.98 ms. The 3.47% delta was
classified `within_noise`; p95 was unavailable for three samples. This is an
after-state datapoint, not a pre/post speedup claim. A first-run Watch frame
and a warmed frame had materially different setup time, so those single-run
durations are not compared.

Render execution completed, but the normal `%LOCALAPPDATA%\Sono-Renderer`
performance and diagnostic stores are unwritable in this environment and
reported access denied. The benchmark used an isolated writable store under
`target/followup27-validation/benchmark-appdata`. All real render, benchmark,
hash, log and extracted image evidence is under the ignored
`target/followup27-validation/` directory; no validation binaries were added
to source control.

## Freeze boundary and remaining debt

For the next stable release, preserve the public `RenderConfig` meanings,
Sonolus LevelData serialization, Watch lifecycle, CLI direct-input behavior,
versioned project/profile/history/manifest formats, job snapshot semantics,
and the contracts in `docs/ARCHITECTURAL_INVARIANTS.md`. New work should extend
those seams. Do not add another Watch evaluator, LevelData graph, project
envelope, cache manager, or editor command framework without a demonstrated
current semantic need.

The largest maintainability cost remains the central GUI and CLI orchestration
files. They should be split only when one workspace or command boundary can be
moved intact and validated independently. The current build is coherent
enough to preserve, but it is not evidence that every requested authoring or
NLE feature exists.
