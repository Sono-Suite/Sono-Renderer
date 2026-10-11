# Sono Renderer

Please scroll down to find the AI disclosure section.

Sono-Renderer renders generic Sonolus Watch-mode gameplay from an engine,
level, resources, and optional media, and also includes Chart Studio plus a
standalone Presentation Studio for ordinary video editing. The render path has
compatibility work and real-engine fixtures for Next Sekai, Next Rush, and
Horizon charts.

## Install a release

The Windows x86_64 release pipeline produces a per-user installer and an
explicit portable ZIP. No official release artifact is published from this
working tree yet. When a tagged release is available, verify the downloaded
installer or ZIP with its accompanying `SHA256SUMS.txt` before use. See
[Install and run Sono-Renderer](docs/INSTALLATION.md) for supported-platform
status, FFmpeg/GPU requirements, portable storage, updates, and uninstall
behavior. The release process and automated acceptance scope are in
[RELEASE_PROCESS.md](docs/RELEASE_PROCESS.md).

If you wish to get a .scp and engine zip, please reference Sono-Server, as Sono-Server has prepackaged .scp files and engine folders! Just zip an engine folder, and you're good :P

The desktop application and command-line renderer use the same rendering
backend:

## Quick Start

For a first render, launch **Sono-Renderer** from the Start menu or double-click
`Sono-Renderer.exe` in a portable folder. Choose **Quick Render**, then select:

- the Sonolus engine ZIP that runs the level;
- the level/chart file;
- the engine resource package, usually SCP or ZIP;
- music when **Include music** is enabled (enabled by default);
- an output MP4 path.

Choose **Check inputs** to verify the selected files and review the shared
compatibility report, then **Queue render**. The app uses the existing
persistent render queue and keeps showing status until it finishes. Input
preflight also checks compatibility findings and whether FFmpeg is already
available; on Windows a missing pinned FFmpeg pair is installed lazily during
export. The success panel provides **Open Video** and **Open Folder** actions.
Projects and profiles are optional; Quick Render uses the same `RenderConfig` and
queue as the advanced workspace.

The packaged Windows app runs without Rust or Cargo. Developers running from a
source checkout can start the same GUI with `cargo run --release -- gui`.

Drop supported files or a folder into the app to discover input roles. Unique
matches are filled automatically; ambiguous matches are shown for you to
choose. For more detail, see [Getting Started](docs/GETTING_STARTED.md).

Authorized single-item URLs can also be inspected and acquired from **Project
/ inputs**. Acquired audio/video is assigned to the existing BGM/MV inputs and
stored locally for offline use. See [URL media acquisition](docs/MEDIA_ACQUISITION.md)
for the platform, playlist, privacy, and editor limitations.

The Welcome screen exposes **Render a Level**, **Create / Edit a Chart**,
**Edit a Video**, **Open a Project**, and **Learn**. Presentation Studio is a
standalone nonlinear editor for ordinary local video, audio, images, and text;
it does not require Sonolus inputs. It provides a multitrack timeline,
continuous preview playback, effects, transitions, keyframes, and MP4 export.
Explicit microphone recording inserts a lossless WAV as ordinary timeline
audio. See [Getting Started](docs/GETTING_STARTED.md),
[Presentation Studio](docs/PRESENTATION_STUDIO.md), and
[microphone recording status](docs/PRESENTATION_STUDIO_AUDIO_INPUT.md).

**Chart Studio** opens or creates generic Sonolus LevelData, discovers
archetypes from the selected engine, supports entity/raw JSON edits with
undo/redo, previews through the production Watch renderer, saves an editable
snapshot in the existing render project, and exports standalone gzip
LevelData. Engine-specific authoring labels are optional and require an exact
content-identity-matched declarative schema. See
[Chart Studio](docs/CHART_STUDIO.md) for supported workflows and current
limits.

The application language can be selected from Home or Settings using native
language names. Initial catalogs are provided for English, Japanese,
Simplified and Traditional Chinese, Korean, Spanish, Brazilian Portuguese,
French, German, and Russian. Current localization covers the primary Home and
workflow entry surfaces; detailed controls may fall back to English, and
non-English text still needs human linguistic review. See
[Getting Started](docs/GETTING_STARTED.md) and the
[localization contributor notes](locales/README.md).

This project also supports direct command-line rendering:

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --start-time 0 --duration 10 --fps 60 --width 1920 --height 1080
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --whole-chart
```

## Build and launch

Sono-Renderer is built from source with the Rust stable toolchain. On Windows,
build and open the desktop application with:

```powershell
cargo build --release --offline # omit --offline on a first build without cached crates
cargo run --release -- gui
```

The CLI is the same executable:

```powershell
target\release\renderer.exe --help
target\release\renderer.exe render-video --help
```

Packaged users can run `Sono-Renderer.exe --version` or
`Sono-Renderer.exe --help`. Cargo's package version is the one version source;
the GUI About panel also shows the source/build identity.

On first Windows export, the renderer looks for a validated shared FFmpeg pair
and otherwise downloads its pinned FFmpeg 8.1 distribution. That first-time
provisioning requires network access. Use `renderer ffmpeg-info` to inspect the
resolved encoder and probe executables. FFmpeg is required for MP4 export.

Windows is the only platform exercised as a production build in this
development cycle. The source has Linux/macOS UI and native-notification
branches, but those targets have not been build- or interactively validated.
Automatic FFmpeg provisioning is Windows-only; the application does not search
`PATH` for FFmpeg on other platforms. Treat Linux/macOS builds as unverified.

## Watch execution modes

Sono VM / interpreter remains the default. `render-video` and `run-watch` accept
`--vm interpreter` or `--vm sono-gcc` (also `--vm=...`). The existing GUI has
the same setting under **Video / Watch execution**.

```text
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --vm interpreter
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --vm sono-gcc
cargo run --release -- run-watch <engine.zip> <level.json.gz> --vm sono-gcc
```

## Watch CPU scheduling

Parallel Watch scheduling is the default. Independent callback batches use a
persistent bounded worker pool and merge their results in source order. Small
batches and callbacks that touch shared state stay on the ordered path. Use
`--watch-workers` to cap worker count, or `--single-threaded` to disable the
scheduler and avoid creating workers:

```text
cargo run --release -- run-watch <engine.zip> <level.json.gz> --watch-workers 4
cargo run --release -- run-watch <engine.zip> <level.json.gz> --single-threaded
cargo run --release -- render-video <engine.zip> <resources.scp> <level.json.gz> <music-file> <output.mp4> --single-threaded
```

The GUI has the same scheduler checkbox. `run-watch` also accepts
`--no-vm-accounting` for production-like CPU measurements without per-operation
diagnostic counters; normal runs keep those counters enabled.

## Shared renderer configuration

The CLI and desktop GUI use the same `RenderConfig` JSON format. In the GUI,
use **Load configuration...** and **Save configuration...**. The saved file
keeps project inputs, timing, resolution/FPS, CPU or WGPU backend, Watch
execution and worker settings, layers, gameplay UI profile data, watermark
settings, and diagnostics together. No GUI settings are stored in a separate
format. See the [complete setting reference, defaults, limits, and frontend
availability](docs/USER_CONFIGURATION.md) and the [Follow-up #8 report](SONO_RENDERER_USER_CONFIGURATION_FOLLOWUP_8_REPORT.md).

CLI-only workflows can start from a template, edit its input/output paths, and
validate it before rendering:

```text
renderer default-config renderer-settings.json
renderer validate-config renderer-settings.json
renderer render-config renderer-settings.json
```

The default template has empty engine, resource, level, and output paths. Set
those before validation. When BGM is enabled, provide a music file. Relative
paths in a config file, including gameplay UI assets/events and watermark
images, resolve from the directory containing that config file. The supported
render values are: dimensions 1..=8192 (MP4 dimensions must be even), FPS
1..=240, nonnegative start time, positive duration, `cpu` or `wgpu` backend,
`interpreter` or `sono-gcc` execution, parallel Watch scheduling with an
optional worker limit of 1..=256, and the existing render-layer toggles.

The shared renderer config also contains cross-platform desktop and Discord
notifications. The desktop **Notifications** tab edits the same JSON settings
used by the CLI. See the [notification guide](docs/NOTIFICATIONS.md) for
backend options, per-destination filters/templates, environment-based bot
credentials, and non-blocking delivery behavior. `render-video` can take a
standalone notification file with `--notifications-config` or disable
notifications for one run with `--no-notifications`.

Gameplay UI and watermark profiles can also be selected directly on the
existing `render-video` command, without removing any of its flags:

```text
renderer render-video engine.zip skin.scp level.json.gz music.mp3 output.mp4 `
  --ui-profile hud.json --watermark-profile watermark.json --no-moving-watermark `
  --video-crf 18 --video-preset medium
```

`--no-watermark` disables both fixed attribution and moving watermark;
`--no-moving-watermark` keeps the fixed attribution. The master switch defaults
to enabled and the moving watermark defaults to disabled. Fixed attribution
text and its bottom-left position remain fixed; size and opacity are
configurable. Custom gameplay UI element layouts, bindings, fonts, and assets
remain profile data, separate from the desktop application's own controls.

Sono-GCC's exact-graph cache remains enabled automatically. The GUI's
**Render / video &rarr; Compiler cache** section and `renderer clear-gcc-cache`
provide cache cleanup; deleting it only causes eligible code to be recompiled.
The **Cache manager** tab and `renderer cache` commands provide shared
inventory, integrity validation, safe removal, explicit size pruning, and
rebuild controls. They only manage generated caches; profiles, settings,
render-job history, source assets, render outputs, and retained `artifacts/`
evidence are outside their scope. See the [cache management guide](docs/CACHE_MANAGEMENT.md)
for cache identity, recovery, and the commands.
The MP4 encoder remains H.264/YUV420p with the existing audio pipeline. Video
quality uses configurable H.264 CRF 0..=51 (default 18) and the x264 speed
presets from `ultrafast` through `veryslow` (default `medium`).

## Persistent render jobs

The desktop **Job manager** tab and `renderer jobs` CLI commands use one
persistent queue. Jobs capture the current `RenderConfig`, report progress and
diagnostics, and preserve output files with no-clobber publication. The
default concurrency is one. See the [job manager guide](docs/RENDER_JOB_MANAGER.md)
for queue commands, recovery, cancellation, and the shared store location.

Completed, failed, and cancelled attempts are retained separately in
**Render history**. The GUI tab and `renderer history` commands can inspect
configuration, output, performance, and diagnostics, and submit a new job
with **Render Again**. See the [render history guide](docs/RENDER_HISTORY.md)
for retention, recovery, and CLI examples.

Export a content-identified record of a completed attempt with
`renderer manifest export <history-id> render.json`. Manifests verify the
resolved assets and effective configuration, compare with later attempts,
and submit a fresh queue job for reproduction. They keep asset paths as
portable hints and do not embed engine, chart, audio, or output binaries. See
the [reproducible render manifest guide](docs/RENDER_MANIFESTS.md) for schema,
verification limits, repair, and CLI usage.

Named full-render profiles save reusable `RenderConfig` snapshots across the
GUI, CLI, and render queue. Use `renderer profiles list` to inspect the shared
catalog, `renderer render-config --profile "High Quality"` to render one, or
`renderer jobs submit --profile "High Quality" --enqueue-only` to queue it.
See [full-render profile documentation](docs/CONFIGURATION_PROFILES.md) for
editing, import/export, validation, and profile-store location.

The Performance / benchmarks tab and `renderer performance` commands share
the same versioned render records. They distinguish queue wait from render
execution, report disjoint export phases separately from overlapping pipeline
and per-frame attribution, and compare matched VM/GCC runs with observed cache
state. See the [performance laboratory guide](docs/PERFORMANCE_LAB.md) for
commands, comparison limits, schema, and retention.

The **Regression Lab** captures and compares real-renderer visual, audio, and
semantic checkpoints. References require explicit creation/replacement, while
normal renders add no regression capture work. See the
[regression laboratory guide](docs/REGRESSION_LAB.md) for the Larp 64x fixture,
layer semantics, CLI/GUI workflows, image artifacts, limits, and comparison
policy.

Sono-GCC is an opt-in hybrid path. It compiles 75 kinds of eligible scalar
operations directly, including arithmetic, math, interpolation, and 32 easing
variants. The other 86 Watch operation kinds can cross an ordered scalar cut:
WatchVm evaluates the operation with authoritative semantics, then a native
parent can consume its exact `f64` result. The cut does not move the operation
into native code or call back from the DLL into WatchVm. Graphs without a safe
native parent, or beyond static region limits, continue entirely in WatchVm.
Elastic easing remains interpreted because native code changed a NaN payload
in differential testing. Watch tracing also forces interpreter execution for
traced callbacks.

Native DLL generation currently requires Windows and an available Rust
compiler (`rustc`). Compile or load failures fall back to the interpreter;
unsupported graph paths always remain there. Selecting an engine while
Sono-GCC is enabled starts background graph loading and precompilation. The GUI
reports cache lookup, compilation, partial coverage, and fallback status;
rendering remains available while compilation runs. GUI precompilation,
`compile-engine <engine.zip>`, and rendering share the exact-graph cache across
processes when the cache directory is writable. Cache identity includes the
graph, backend/runtime ABI, compiler identity, target, and build flags.

```text
cargo run --release -- compile-engine <engine.zip>
```

The current operation inventory and per-operation implementation/test status
are in [the coverage matrix](SONO_GCC_COVERAGE.md). The implementation report
records real-engine parity, cache timings, scalar-cut counts, and remaining limits.
The [performance report](SONO_GCC_PERFORMANCE_REPORT.md) contains repeatable
real-chart benchmarks, cache-hit breakdowns, and the current export bottleneck.

In the latest repeated release benchmarks, GCC reduced isolated Watch time on
five of six real fixtures and tied at millisecond resolution on LIMBO. The
20-frame Larp SFX export remained 7.4% slower end to end under GCC, so Sono-GCC
is intentionally not the default. See the [implementation report](SONO_GCC_IMPLEMENTATION_REPORT.md)
for the tested operation subset and parity matrix, and the [performance report](SONO_GCC_PERFORMANCE_REPORT.md)
for current timings and benchmark details.

## Documentation

- [UI workflow and information architecture](UI_INFORMATION_ARCHITECTURE_OVERHAUL.md)
- [Workflows, layouts, commands, and shortcuts](docs/WORKSPACE_COMMANDS.md)
- [User-facing getting started guide](docs/GETTING_STARTED.md)
- [Follow-up #36 repository feature-existence audit](FEATURE_EXISTENCE_AUDIT.md)
- [Follow-up #39 release-candidate certification](RELEASE_CANDIDATE_CERTIFICATION.md)
- [Follow-up #40 post-certification gap closure](POST_CERTIFICATION_GAP_CLOSURE.md)
- [Current product limitations](docs/KNOWN_LIMITATIONS.md)
- [Follow-up #38 command palette, shortcuts, and workspaces](SONO_RENDERER_FOLLOWUP_38_REPORT.md)
- [Command and workspace user guide](docs/WORKSPACE_COMMANDS.md)
- [Current architecture, invariants, and freeze boundary](docs/ARCHITECTURE.md)
- [General platform and Sonolus domain boundaries](docs/GENERAL_PLATFORM_ARCHITECTURE.md)
- [Hosted admin and operations console](docs/ADMIN_CONSOLE.md)
- [Follow-up #46 modular architecture report](SONO_RENDERER_FOLLOWUP_46_REPORT.md)
- [Architectural invariants to preserve](docs/ARCHITECTURAL_INVARIANTS.md)
- [Follow-up #27 architecture consolidation report](SONO_RENDERER_FOLLOWUP_27_REPORT.md)
- [Follow-up #21 maintainability report](SONO_RENDERER_FOLLOWUP_21_MAINTAINABILITY_REPORT.md)
- [Portable project workflow](docs/PROJECT_WORKFLOW.md)
- [Post-integration Follow-up #6 report](SONO_RENDERER_POST_INTEGRATION_FOLLOWUP_6_REPORT.md)
- [Shared settings and validation](docs/USER_CONFIGURATION.md)
- [Full-render profiles](docs/CONFIGURATION_PROFILES.md)
- [Render jobs and queue recovery](docs/RENDER_JOB_MANAGER.md)
- [Persistent render history and Render Again](docs/RENDER_HISTORY.md)
- [Reproducible render manifests](docs/RENDER_MANIFESTS.md)
- [Follow-up #22 implementation and validation report](SONO_RENDERER_FOLLOWUP_22_REPORT.md)
- [Post-integration Follow-up #7 report](SONO_RENDERER_POST_INTEGRATION_FOLLOWUP_7_REPORT.md)
- [Final mega-integration #2 stability report](SONO_RENDERER_FINAL_MEGA_INTEGRATION_2_REPORT.md)
- [Performance history and benchmarks](docs/PERFORMANCE_LAB.md)
- [Visual, audio, and semantic regression laboratory](docs/REGRESSION_LAB.md)
- [Post-integration Follow-up #5 report](SONO_RENDERER_POST_INTEGRATION_FOLLOWUP_5_REPORT.md)
- [Engine / project compatibility analyzer](docs/COMPATIBILITY_ANALYZER.md)
- [Post-integration Follow-up #4 report](SONO_RENDERER_POST_INTEGRATION_FOLLOWUP_4_REPORT.md)
- [Structured render diagnostics](docs/RENDER_DIAGNOSTICS.md)
- [Gameplay UI profiles and visual validation](ui-profiles/README.md)
- [Watermark profiles](watermark-profiles/README.md)
- [Notifications and Discord](docs/NOTIFICATIONS.md)
- [Troubleshooting and current limitations](docs/TROUBLESHOOTING.md)
- [Sono-GCC coverage](SONO_GCC_COVERAGE.md), [implementation report](SONO_GCC_IMPLEMENTATION_REPORT.md), and [performance results](SONO_GCC_PERFORMANCE_REPORT.md)

The command and option reference is available from `renderer --help` and each
subcommand's `--help` output. CLI examples assume the executable is available
as `renderer`; on Windows, use `target\release\renderer.exe` or run commands
through `cargo run --release --`.

legal jargan lol:

I do not claim to have written any of the code in this project, as I do not personally know the programming language Rust. This project is AI assisted "to the absolute extreme", and if you don't like that, it's fine. However, please make the distinction that I am not going to turn my YouTube content flow and actual creative works into AI slop machine. AI content slop is not something that I endorse in any way, shape or form. Please understand my intention with this project is not to claim that I wrote the code in this project, but to provide a tool that helps automate the chart production process much much quicker.

Acknowledgements:

Zihad - Sonolus Renderer Code provided as reference

LittleYang0531 - More rendering code provided as reference

qwewqa / Hyeon - Next Sekai Preview implementation for development reference

Burrito (Sonolus) - literally the entire reason this exists, W

Being mentioned here does not necessarily mean that they have endorsed this project.
Credit is given to acknowledge the people who made related works that contribute to its testing.

All developers whose code was provided to AI as a development reference were made explicitly aware and gave permission for my use case of their code. Again, this permission does not imply endorsement of Sono-Renderer.

This program was built and tested against Sonolus v1.1.4.
