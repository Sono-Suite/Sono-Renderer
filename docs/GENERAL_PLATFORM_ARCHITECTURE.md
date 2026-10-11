# General platform and Sonolus domain boundaries

This document records what the current repository implements and the
constraints future work should follow. It does not claim that future audio,
image, animation, development, or plugin products already exist. The broader
code map and persistence schema table remain in [Architecture](ARCHITECTURE.md).

## Current shape

The repository is one Rust package and one crate, not a set of independently
compiled platform/domain crates. `src/lib.rs` exposes subsystem modules;
`src/main.rs` owns the Clap command tree and CLI dispatch; `src/gui.rs` owns
the eframe desktop shell and connects workflow UI to library services. This
is a real separation of responsibilities, but Rust's crate boundary does not
enforce the dependency direction between sibling modules.

The useful current dependency shape is:

```text
Application shell (main.rs / gui.rs)
       |                         |
       v                         v
Render workflows             Presentation workflow
       |                         |
RenderConfig / RenderJob     CompositionProject / media graph
       |                         |
       v                         v
WatchRuntime + WatchVm       FFmpeg media decode / timeline playback
       |                         |
       +------> shared filesystem, FFmpeg process, cache and UI services
```

This is a boundary inside one application, not a generic editor framework or
plugin host. Presentation Studio's `CompositionProject` is already usable
without Sonolus input: its production module declares and implements that
boundary, and it does not import Watch, engine, LevelData, or `RenderConfig`
types. The normal Sonolus render route remains separately available.

## Current ownership

| Area | Current owner | Boundary and limits |
|---|---|---|
| Application entry and CLI | `main.rs` | One Clap tree and dispatch module; existing nested command groups provide some organization, but there is not a command-registration/plugin API. |
| Desktop shell and workflows | `gui.rs`, `workspace.rs` | The shell routes among concrete workflows and persists panel/workspace preferences. Routes are finite enums, not dynamically registered editors. |
| Sonolus render subject | `project.rs` | `RenderProject` refers to engine, level, resources, music, output defaults, profile identity, and project metadata. It is intentionally Sonolus-shaped. |
| Standalone edit document | `composition.rs` | `CompositionProject` owns ordinary media references, tracks, clips, effects, transitions, captions, and keyframes. It has its own versioned schema and recovery path. |
| Canonical Sonolus level | `formats.rs` | `LevelData` is the renderer and Chart Studio semantic representation; it is not a general-purpose document model. |
| Chart authoring | `chart_studio.rs` | `ChartDocument` edits canonical LevelData and has chart-local authoring/undo behavior. |
| Generic decoded audio summaries | `media_audio.rs` | Bounded PCM WAV inspection and FFmpeg waveform summaries used by ordinary media workflows. It does not own device capture or audio playback. |
| Microphone capture | `audio_input.rs` | Device enumeration, explicit capture, staged WAV writing and recovery. Finalized WAV becomes ordinary composition media. Existing public waveform helper paths re-export `media_audio` for compatibility. |
| Sonolus resource packages | `formats.rs`, `presentation_assets.rs` | Package/resource semantics and renderer/gameplay resource identity remain domain-owned; there is no universal asset payload model. |
| Shared cache administration | `cache_manager.rs` | Safe inventory/retention management; cache identity and correctness remain with the producer, including Sono-GCC and derived media work. |
| Render execution | `render.rs`, `video_export.rs`, `offline.rs`, `watch_runtime.rs`, `runtime.rs`, `gpu_render.rs` | One Sonolus execution path from Watch lifecycle through frame preparation to output. |
| Render jobs | `job_manager.rs`, private `job_config.rs` | `RenderJobManager` is specifically a render queue over captured `RenderConfig`, with lifecycle/recovery, output reservations, history projection, and notification integration. It is not a generic job framework. |
| Render evidence | `performance.rs`, `render_diagnostics.rs`, `render_history.rs`, `render_manifest.rs`, `regression.rs` | These authoritative records describe renderer attempts, GCC/cache identity, render workloads, and output evidence. They are not flattened into universal job telemetry. |
| Profiles | `config_profiles.rs` | Reusable render configuration, not a project or workspace document. |
| Presentation time | `composition.rs`, `timeline_playback.rs` | Composition seconds, source-media mapping, and playback sample clock belong to the NLE. They do not use Watch offset. |
| Sonolus time | `export_timeline.rs`, `offline.rs` | The gameplay/media mapping is `media_time = watch_time + bgmOffset`; output frame/sample mapping stays in the render timeline. |
| Commands and shortcuts | `workspace.rs` | Stable command identifiers and shortcut resolution support current UI surfaces. They do not currently provide a universal document command/undo framework. |
| Persistence | Owning subsystem modules | Each persistent format has an owner for schema validation, migration, and writing. Project formats for render inputs and composition documents remain intentionally distinct. |
| Notifications | `notifications.rs` | Render/job events feed optional delivery. Delivery is auxiliary and cannot redefine render success. |
| Hosted administration API | `admin.rs`, `web/` | Optional same-process hosted control surface; it calls existing queue/evidence/cache/profile owners through versioned DTOs. It is disabled by default and never owns render semantics. |
| FFmpeg/process boundary | `ffmpeg.rs`, `export_control.rs`, owning media/render callers | Tool resolution and child-process controls are shared where semantics match; command construction and lifecycle remain with each operation. |

## Dependency rules

### Allowed

- The GUI and CLI may orchestrate multiple domain workflows and shared
  application services.
- The optional hosted admin API may expose stable, redacted DTOs and invoke the
  same authoritative operations as the GUI/CLI. Its frontend must not
  calculate renderer evidence or become a required platform dependency.
- Sonolus render and authoring modules may consume filesystem, serialization,
  media-processing, cache administration, and application UI infrastructure.
- A domain adapter may translate its domain data into a shared media input,
  output request, or presentation object when the target representation has
  matching semantics.
- Shared infrastructure may expose narrow operations that have at least two
  demonstrated callers with the same behavior. `media_audio` is one such
  boundary: generic composition waveform/PCM inspection no longer depends on
  the microphone-capture lifecycle.
- A legacy module path may re-export a moved helper to avoid breaking Rust
  callers while its implementation has one owner.

### Do not add

- Watch, `EngineData`, `LevelData`, `bgmOffset`, or `RenderConfig` as a
  prerequisite of generic media, audio, composition, workspace, or filesystem
  code.
- Sonolus SFX/resource/skin types as the native model of ordinary media.
- A mandatory fake `RenderProject` around an unrelated document.
- A universal asset payload enum, generic job system, universal project
  envelope, plugin registry, service locator, or editor framework without
  multiple concrete consumers that share the semantics.
- A second Watch evaluator, gameplay scheduler, rendering clock, performance
  timer, cache identity, history store, or CLI command vocabulary.
- Genericizing Sonolus semantics until they lose meaning. Watch rules,
  LevelData, resource package rules, and Sono-GCC eligibility/fallback belong
  to the Sonolus domain.

## Domain and shared boundaries

### Sonolus domain

Engine packages and SCP resources, LevelData, Watch graph/lifecycle, VM
semantics, Sono-GCC's optional eligible scalar execution, compatibility
analysis, Sonolus project resolution, and gameplay render adapters are
first-class domain-owned behavior. `WatchVm` remains the semantic oracle;
Sono-GCC remains optional and must preserve the existing fallback rules.
Diagnostics and performance records may carry this domain's backend/cache
details without forcing those fields onto unrelated future work.

### Shared or already independent today

- `CompositionProject` and its media/timeline/compositor path are independent
  of Sonolus. Gameplay can enter only through an explicit source workflow;
  ordinary media is not required to construct an engine or Watch runtime.
- FFmpeg resolution, subprocess control, generic filesystem operations,
  serialization helpers, localization, workspace command identifiers, and
  cache administration serve multiple current application workflows, though
  they remain modules in this crate.
- `media_audio` owns generic WAV inspection and waveform summaries. Microphone
  capture remains in `audio_input`; its final WAV is imported as ordinary
  project media. This boundary does not create a general audio engine.
- Composition media sources and Sonolus package resources remain separate
  payload models. Both can use filesystem paths and content identity where
  appropriate without pretending their import, lifetime, or validation rules
  are identical.

## Application, documents, and assets

There is no universal workspace/document envelope. The application shell can
route between the current render, chart, and presentation experiences. The
persisted `WorkspaceId` concerns panel arrangement; `WorkflowId` concerns the
active app route. Sonolus render projects and standalone composition projects
are separate document types with separate loaders and schema histories.

There is also no universal asset registry. `CompositionProject` owns its
media references and source metadata. Sonolus resource/package code owns
resource validation and identity. Derived thumbnails, waveforms, proxies,
compiled artifacts, and other caches remain disposable and are owned by the
subsystem that knows their cache key. Future work should extract only shared
identity/location/ownership operations demonstrated by more than one real
asset family; it should not unify payload interpretation.

## Time domains

Numeric values do not imply interchangeable clocks. The current repository
has distinct semantics for:

- wall-clock creation/start/end timestamps;
- monotonic elapsed duration in render performance records;
- Watch time and its `bgmOffset`-based media mapping;
- composition seconds and clip/source mapping;
- audio device/sample positions and playback clock;
- subtitle/media timestamps;
- BPM/beat observations and their advisory phase.

Keep conversions at explicit domain boundaries. A generic composition must
not use `bgmOffset` as its native clock; a Watch render must not replace its
established mapping with a generic editor clock. The media-audio extraction
does not change timing or sample conversion.

## Commands, undo, and execution

`workspace.rs` is the current owner of stable UI command IDs and shortcut
resolution. It is not an editor mutation/undo abstraction. Chart authoring
and composition editing have document-specific history because their
mutations and persistence semantics differ. Rendering, benchmarking, cache
maintenance, and analysis are operational jobs, not automatically undoable
document edits.

The current persistent queue is specifically `RenderJobManager`. It captures
resolved render configuration and handles render lifecycle, cancellation,
retry/recovery, output collision rules, and result projection. Do not rename
it conceptually into a general executor until another concrete workload
requires shared scheduling semantics and the compatibility story is clear.

## Telemetry, history, and tests

`RenderPerformanceRecord`, render history, diagnostics, manifests, and
regression captures are authoritative for their render-specific evidence.
Other future domains can own domain-specific evidence or add typed extensions
if a real shared schema emerges; they should not invent another stopwatch for
renderer concepts or force all evidence into render-shaped records.

Tests are intentionally grouped by behavior: ordinary module tests,
subsystem/integration tests, real Sonolus fixtures and VM/GCC differential
checks, visual/semantic regression fixtures, and performance benchmarks.
The Watch VM remains the Sonolus oracle. Generic media tests should stay
independent of an engine fixture where possible. Architecture tests are not a
replacement for output and semantic parity tests.

## CLI, GUI, and extensibility

The CLI currently has a single top-level Clap command declaration and
centralized dispatch in `main.rs`, with some existing nested domains. The GUI
has one concrete shell in `gui.rs` and finite workflow/panel identifiers in
`workspace.rs`. This supports the product that exists today. It is not yet a
modular command-registration or dynamically extensible workspace system.

Future modularization should begin when a second independently owned domain
actually needs command ownership or shell registration. At that point prefer
static Rust modules and narrow interfaces. Do not add dynamic loading, global
mutable registries, or placeholder workspaces merely to advertise future
extensibility.

## Persistence and compatibility

Persistent formats keep one owner for load, validation, migration, and save.
The Sonolus render project and Presentation composition document remain
separate formats; queue, history, profiles, manifests, telemetry, and caches
retain their current owners and versions. `media_audio` produces derived
summaries and adds no durable schema. Existing `audio_input` public waveform
helper paths re-export the shared implementations for source compatibility.

When introducing a persistent type, version it and test the actual migration
fixtures. Do not migrate data only to make a conceptual architecture more
uniform. Preserve unknown fields where the existing schema promises to do so.

## Future contract

1. A future audio workspace must not depend on Watch or Sonolus resource
   types. Its actual media/recording semantics should have an owner before
   shared infrastructure is extracted.
2. An image editor must not require `EngineData`, `LevelData`, or a render
   project merely to open an image.
3. A generic timeline owns its own clock and source mapping; Watch time and
   `bgmOffset` stay behind a Sonolus adapter.
4. A generic job must not require a Sonolus render configuration. Share only
   scheduling behavior proven common across real job types.
5. Sonolus adapters may consume platform services, but platform services may
   not import Sonolus concepts to satisfy a generic caller.
6. Keep `WatchVm` authoritative. New previews, render modes, tests, or domain
   adapters consume the existing runtime rather than reimplementing Watch.
7. Keep render telemetry and history authoritative for renders. Extend or
   relate them only when another domain has a demonstrated, compatible need.
8. Keep each persistent schema's loader/migrator/writer in one owning
   subsystem and preserve user data across architecture changes.
9. Favor a direct dependency in this single crate until an actual boundary
   requires a crate split. Folder moves and generalized traits are not goals.

## Remaining coupling and pressure points

- Sibling modules are not compile-time isolated because this remains one
  crate; dependency direction is currently established by ownership, API
  visibility, tests, and documentation rather than crate-level enforcement.
- `main.rs` contains the CLI command tree and much orchestration; additional
  domain growth may justify moving command definitions/dispatch into
  domain-owned modules, but no hypothetical command API is added here.
- `gui.rs` remains a large concrete desktop frontend. Current standalone
  Presentation Studio separation is in its model/runtime path, not a
  separately compiled UI plugin boundary.
- `RenderConfig` and the render job manager intentionally remain renderer
  concepts. Their names or records should not be presented as generic app
  infrastructure.
- Composition's media source model and Sonolus's resource model are separate;
  no shared asset catalog currently provides common durable references.
- Render telemetry/history/diagnostics are render-oriented by design.

These are known limits of the current product, not claims of completed future
platforms. Revisit a boundary only when concrete second consumers demonstrate
that semantics are truly shared.
