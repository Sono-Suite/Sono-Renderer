# Follow-up #46 — modular platform architecture report

## Scope and baseline

This was an architecture boundary pass, not a feature expansion. I inspected
the current Rust modules, current architecture/invariant docs, and the actual
CLI, GUI, render, composition, audio, workspace, and persistence owners before
editing production code.

The workspace was already heavily modified and contained many untracked
feature/report files before this pass. I preserved that state; this report
does not claim a clean Git baseline. The preceding #45 verification report
and its full offline test run are the pre-change behavioral baseline available
in this workspace. Since then, the only known renderer-source change before
#46 was an explanatory comment in `sono_gcc.rs`; no timed renderer behavior
changed. The #45 report records its build/test outcomes and notes that its
shared-host performance comparison was not conclusive.

The current suite was also run after the production boundary change; exact
results appear below. The only production behavior code moved was the
existing bounded PCM WAV inspection and FFmpeg waveform-summary implementation
from the microphone module into `media_audio.rs`. The byte/sample algorithms,
FFmpeg arguments, validation, and result shapes were preserved. No render
pipeline, Watch, configuration, persistence schema, cache identity, or job
lifecycle was changed.

## Current architecture before this pass

- One Cargo package and one Rust crate expose subsystem modules through
  `src/lib.rs`; sibling-module dependency direction is not enforced by crate
  boundaries.
- `src/main.rs` owns the single Clap command tree and CLI dispatch.
- `src/gui.rs` owns the eframe shell and its concrete workflow UI.
- `RenderProject` is the Sonolus render-input document; `CompositionProject`
  is an independent Presentation Studio document.
- `formats::LevelData` is canonical for Sonolus chart authoring and render
  input. `ChartDocument` edits that representation.
- `WatchRuntime` owns Watch lifecycle/scheduling and `WatchVm` evaluates Watch
  operations. Sono-GCC is optional acceleration with the established VM
  fallback/oracle behavior.
- `RenderJobManager` is a render queue over captured `RenderConfig`, not a
  general task executor. Render history, performance, diagnostics, and
  manifests are likewise render-oriented authoritative records.
- Composition media/timeline/compositor code already works without Watch,
  engine, LevelData, or `RenderConfig`. Ordinary composition media and
  Sonolus package resources have separate models.
- The concrete accidental coupling found was generic composition waveform
  generation and PCM WAV inspection being implemented in `audio_input.rs`, a
  microphone-capture subsystem. Composition called into that module for
  ordinary imported media.

## Module-by-module decisions

| Module | Current state and coupling | Decision / production change | Evidence and remaining debt |
|---|---|---|---|
| 0. Baseline and map | One crate; `ARCHITECTURE.md` and invariants already map actual owners, Watch flow, persistence, and workers. | **IMPLEMENTED** — used the current repository and prior #45 validation as baseline; documented actual boundaries in this report and the new platform architecture guide. | Current full suite and checks below. The pre-existing dirty workspace limits Git-diff-based baseline reconstruction. |
| 1. Dependency boundaries | Composition is already Sonolus-independent. One generic media-audio helper dependency pointed from composition to microphone capture. | **IMPLEMENTED** — moved shared PCM/WAV and waveform behavior to `media_audio`; composition now depends on that neutral module. | Searches show no `audio_input::` calls remain in production `composition.rs`; legacy public helper paths re-export the shared code. Single-crate sibling dependencies remain unenforced. |
| 2. Application/workspace | `WorkflowId` and `WorkspaceId` describe current routes and panel layouts. Render and composition documents are separate. There is no universal project/workspace document. | **ALREADY SATISFIED** for current workflows. No universal workspace abstraction added. | Composition’s standalone tests cover documents without Sonolus state. A future document registry is not justified by current consumers. |
| 3. Asset/resource foundation | Composition owns media references; Sonolus package/resource loaders own package identities and validation. There is no universal payload/asset catalog. | **IMPLEMENTED** — extracted only PCM/WAV inspection and waveform summaries because both generic composition and capture finalization use the same semantics. | `audio_input` still owns device/capture/recovery; `media_audio` has no device or Sonolus imports. No universal asset model was added. |
| 4. Time foundation | Watch/media offset, composition/source time, sample clocks, elapsed duration, wall timestamps, and beat analysis have distinct owners. | **ALREADY SATISFIED** — preserved all existing mappings; no numeric type conversion/rewrite was warranted. | `export_timeline.rs` remains authoritative for `media = Watch + bgmOffset`; composition uses its timeline/source mapping and playback sample clock. This change does not touch timing. |
| 5. Command/action/undo | `workspace.rs` owns command IDs/shortcuts; Chart Studio and composition each use document-specific undo/history. Operational render/cache work is not document undo. | **ALREADY SATISFIED** — no general command transaction or universal undo system added. | A shared command API has no second demonstrated document implementation with matching mutation semantics. |
| 6. Media/processing | FFmpeg/process control is shared, while composition and gameplay rendering own their graph/semantics. Generic PCM/waveform code was placed in microphone capture. | **IMPLEMENTED** — shared media inspection moved to `media_audio`; encoder, compositor, game renderer, and audio mixer remain where their semantics belong. | Focused media/capture/composition tests pass. No renderer output path was rewritten. |
| 7. Job/execution | `RenderJobManager` is explicitly a persistent render queue carrying `RenderConfig`; it also handles render-specific retry/recovery/finalization. | **INTENTIONALLY SONOLUS/RENDER-SPECIFIC** — kept authoritative; no speculative generic job framework. | Existing job tests are part of the full suite. A second concrete job class is required before extracting common scheduling contracts. |
| 8. Telemetry/history/diagnostics | `RenderPerformanceRecord`, `RenderHistoryRecord`, diagnostics, manifests, and regression records encode render-specific workload/backend/cache facts. | **INTENTIONALLY RENDER-SPECIFIC** — preserved these sources of truth. | Full test suite covers their stores and comparison rules. No duplicate timer or telemetry model introduced. |
| 9. Testing foundation | Unit, subsystem, real-engine, VM/GCC parity, integration, and regression tests are separate by semantic purpose. | **ALREADY SATISFIED** — retained domain-specific test organization; added focused media-audio coverage. | `cargo test --offline` result below. Real-FFmpeg tests remain ignored unless their external fixture/tool conditions are enabled. |
| 10. CLI foundation | One `main.rs` Clap tree and dispatch, with some nested action groups. No domain registration interface. | **NOT JUSTIFIED** to split during this pass. | No CLI behavior or command ownership change was needed; modular registration for hypothetical future domains would be speculative. |
| 11. GUI/workspace shell | One concrete eframe shell with finite current workflow routes and persistent panel layout. | **ALREADY SATISFIED** for the actual product; no placeholder/plugin workspace system added. | GUI tests are included in the full suite. `gui.rs` remains a large frontend and may need domain-owned submodules if new concrete domains arrive. |
| 12. Extensibility seams | Static Rust modules and explicit domain adapters exist; no dynamic plugin system. | **NOT JUSTIFIED** to add registries, service locator, or dynamic loading. | Future extensions should begin with direct modules and narrow APIs until there are multiple actual implementations. |
| 13. Sonolus boundary | Engine/LevelData/Watch/VM/GCC/resource/compatibility/render semantics are intentionally specialized. | **INTENTIONALLY SONOLUS-SPECIFIC** — explicitly documented as a domain, without weakening it. | Full suite includes real-engine and Watch/GCC tests; see exact run result. |
| 14. Persistence/schema compatibility | Each persistent store has one owning loader/validator/migrator/writer. Render and composition projects remain separate schemas. | **ALREADY SATISFIED** — no new durable representation; corrected the architecture doc’s `.sonocomp` version from 20 to the current 21. | No persistence schema changed. Existing migration/atomic-save tests pass in the full suite. |
| 15. Architectural enforcement | Module ownership/API visibility is present, but sibling imports are not isolated by crates. | **DEFERRED** — no brittle source-text architectural test or heavyweight framework added. | A future crate split or dependency checker is justified only if real independent domains need stronger compile-time isolation. |
| 16. Compatibility/parity | Existing tests exercise Sonolus semantics, VM/GCC fallback/parity, compositor, queue/config, persistence, CLI, and GUI. | **TESTED** — full current suite run after the extraction. | This small media-only change does not alter Sonolus paths; exact evidence below. |
| 17. Performance gate | Existing Performance Lab is authoritative; its latest prior comparison was inconclusive under a shared host. | **NOT JUSTIFIED** to benchmark renderer timings for a code move outside the render hot path. | No new timer or render-path edit was made. Same waveform algorithms and FFmpeg invocation were moved intact; no speedup claim is made. |
| 18. Documentation/future contract | Existing architecture docs covered renderer and Sonolus systems but did not state the general-platform dependency contract. | **IMPLEMENTED** — added `docs/GENERAL_PLATFORM_ARCHITECTURE.md`, linked it from README/architecture, and corrected the composition schema version. | Documentation distinguishes current systems from capabilities this architecture merely enables later. |
| 19. Final classification | Some broad modularity exists at module/workflow level, not as independently compiled plugin domains. | **IMPLEMENTED** — classification is explicit in this report and documentation. | No claim is made that future workspaces, universal assets/jobs/projects, or plugins exist. |

## Production change

Added `src/media_audio.rs` as the owner for:

- fixed-layout signed 16-bit PCM WAV inspection;
- bounded PCM WAV peak extraction;
- bounded, streaming FFmpeg waveform summaries;
- the ordinary WAV summary consumed when a captured take becomes media.

`composition.rs` now depends on `media_audio` for ordinary media waveforms and
recorded WAV import. `audio_input.rs` remains the owner of CPAL device
enumeration, capture, staging, WAV writing, interruption recovery, and
preferences. Its previous public waveform helper paths are re-exported from
`media_audio` to preserve Rust API compatibility. There are no new project,
asset, queue, telemetry, plugin, or persistence abstractions.

The other production-text change is one module declaration in `lib.rs`.
`docs/ARCHITECTURE.md` now records `.sonocomp` schema 21, matching
`COMPOSITION_SCHEMA_VERSION`; that is a documentation correction, not a
schema migration.

## Validation and compatibility evidence

### Baseline

The preceding #45 verification report is the pre-change source baseline. It
records a complete offline test run (419 library tests, 27 CLI tests, 101
renderer integration tests, 9 scheduler tests; eight ignored for optional
tools/fixtures), `cargo fmt`, `cargo check`, Clippy, and release build. It
also records that the shared-host performance comparison was inconclusive.
The complete current suite was rerun after this boundary change rather than
using the old count as final evidence.

### This pass

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | **Pass**; formatter emitted only the existing Windows path canonicalization warning. |
| `cargo test --offline media_audio::` | **Pass** — 2 new media-audio tests. |
| `cargo test --offline audio_input::tests` | **Pass** — 6 capture/finalization tests, including finalized take to ordinary composition media. |
| `cargo test --offline composition::tests` | **Pass** — 63 passed, 2 ignored because those require an installed FFmpeg/FFprobe pair and real media fixtures. |
| `cargo check --offline` | **Pass**. Windows build script warns that `rc.exe` is unavailable, so executable version metadata is not embedded. |
| `cargo test --offline` | **Pass** — library 421 passed / 8 ignored; CLI 27 passed; `arctan2` 3 passed; real-fixture integration 101 passed; scheduler 9 passed; doc tests 0. No failures. |
| `cargo clippy --offline --all-targets` | **Pass** with the repository's existing warning set (195 lib-test and 14 bin-test warnings were reported; no warning cleanup was in scope). Windows build script reports `rc.exe` unavailable. |

After that full run, the private `audio_input::read_wav_format` delegating
wrapper was removed so capture finalization calls the shared parser directly.
The exact final source then passed `cargo test --offline audio_input::tests`
(6/6), `cargo check --offline`, and `cargo fmt --all -- --check`.

No real microphone/device test was performed or needed for this change. No
packaged-artifact test was performed. The composition FFmpeg end-to-end cases
were ignored because the test command did not enable their installed-tool
fixture preconditions. Ordinary composition media imports continue to use the
same FFmpeg resolution and decoder invocation. No performance benchmark was run because
the render hot path and algorithm were unchanged; no performance improvement
is claimed.

## What exists now

- A domain-independent module owns shared PCM WAV inspection and waveform
  summarization; Presentation Studio no longer imports microphone-capture
  implementation code for ordinary media.
- Sonolus execution/authoring/rendering remains first-class and specialized.
- The application has separate concrete Sonolus render and standalone
  composition documents/workflows, but no universal document/asset/job or
  plugin framework.
- Shared time, persistence, telemetry, history, diagnostics, and Watch
  authority remain in the existing owners.

## What this enables later

- Future independent workflows can reuse concrete services where semantics
  match, and can add domain-owned documents/media/jobs without wrapping them
  in Sonolus models.
- A later crate or command/UI module split can be considered when a second
  real domain demonstrates a need for stronger ownership enforcement.
- Future audio/image/development tools remain future work. This pass did not
  implement them or prove their product workflows.

## Remaining debt and risks

- One crate means module-level dependency direction is not compiler-enforced.
- `main.rs` centralizes CLI commands and `gui.rs` centralizes the concrete
  shell/frontend. They are known pressure points, not immediate blockers.
- No shared asset catalog exists; composition media and Sonolus resource
  packages remain separate on purpose.
- The render queue, performance records, history, diagnostics, and manifests
  remain render-specific.
- No apples-to-apples Performance Lab comparison was meaningful for this
  non-hot-path move.

Future work should favor domain-owned modules, preserve `WatchVm` as the
Sonolus semantic oracle, and extract only demonstrated shared behavior. See
[General platform architecture](docs/GENERAL_PLATFORM_ARCHITECTURE.md) for
the dependency contract.
