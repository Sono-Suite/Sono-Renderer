# Shared User Configuration

Sono-Renderer uses `render::RenderConfig` for the CLI, desktop GUI, preview API,
and video-export API. The desktop **Load configuration...** and **Save
configuration...** actions read and write this same JSON object. The CLI's
`default-config`, `validate-config`, and `render-config` commands use the same
format.

```text
renderer default-config renderer-settings.json
# Edit engine, resources, level, music, and output in the JSON file.
renderer validate-config renderer-settings.json
renderer render-config renderer-settings.json
```

The template intentionally has empty required input and output paths. The
validator checks numeric settings, required files, BGM requirements, MP4
dimensions, and enabled UI/watermark assets before rendering begins.

Named full-render profiles are additive snapshots of this same `RenderConfig`.
The desktop profile selector edits through the existing tabs, while the CLI
provides `profiles` management and explicit `--profile` use for validation,
rendering, and queued jobs. See [full-render profiles](CONFIGURATION_PROFILES.md)
for persistence, import/export, selection, and snapshot behavior.

## Path resolution and persistence

Paths in a configuration file may be absolute or relative. Relative paths are
resolved against the directory containing that config file. This includes
embedded gameplay UI image/font assets, event files, and watermark assets.
Loading a standalone gameplay UI or watermark profile through the GUI or CLI
resolves its profile-relative resources and stores explicit paths in the shared
configuration, so saving that config elsewhere does not reinterpret the
profile's relative paths.

Configuration persistence is explicit: use the GUI load/save actions or keep a
JSON file and pass it to the CLI. The application did not previously maintain
an automatic settings file, so no existing settings file is migrated or
overwritten. Saved config files keep the flat field layout and now declare
`format: "sono-renderer-config"` and `schema_version: 1`. Original unversioned
flat files remain readable with the established serde defaults; saving one
through the renderer upgrades its markers without changing its settings.
Unknown formats, incomplete markers, and future schema versions fail with an
error instead of being silently interpreted. Saves stage and flush a
same-directory temporary file before atomic replacement.

## Setting reference

The JSON configuration is the complete surface in both CLI and GUI. Existing
CLI flags remain supported for their original workflows; the GUI presents the
common choices as controls and saves all fields, including fields that are
profile data rather than single widgets.

| Category / JSON fields | Default | Valid values and interactions | CLI | GUI |
| --- | --- | --- | --- | --- |
| Project: `engine`, `resources`, `level` | Empty in a template | Required accessible files for video export | `render-video` positionals or config file | Project / inputs |
| Resources: `skin`, `resource_overrides.{particles,effects,background,particle_name}` | Engine defaults; no overrides | Resource package paths must exist when supplied | Config file; existing `--particle-resources`, `--particle-name` | Project / inputs |
| Audio / MV: `music`, `mv`, `mv_enabled`, `mv_background` | No music/MV; MV enabled; 100 percent background brightness | BGM needs music when its layer is enabled. MV brightness is 0..=100 and is independent of media timing. | Existing positionals, `--mv`, `--no-mv`, `--mv-background`; config file | Project / inputs and Render / video |
| Output: `output`, `video_crf`, `video_preset` | No path; CRF 18; `medium` | Current container/video codec remain MP4/H.264 with YUV420p. CRF is 0..=51 (lower means higher quality and larger output); preset is one of `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, or `veryslow`. The current audio mix/codec pipeline remains fixed. | Existing output positional; new `--video-crf` and `--video-preset`; config file | Project / inputs and Render / video |
| Timing: `start_time`, `duration`, `whole_chart`, `fps` | 0s, 2s, false, 12 FPS in `RenderConfig`; GUI first-run keeps its existing 20s / 60 FPS defaults | Start is finite and nonnegative; duration is positive (the field remains required when whole-chart inference is selected); FPS is 1..=240. | Existing `--start-time`, `--duration`, `--whole-chart`, `--fps`; config file | Render / video |
| Picture: `width`, `height`, `backend` | 640x360, WGPU in `RenderConfig`; GUI first-run keeps its existing 1920x1080 | Each dimension is 1..=8192. MP4 needs even dimensions. Backend is `cpu` or `wgpu`. | Existing width/height/backend options; config file | Render / video |
| Layers: `layers.{particles,sfx,bgm}` | All enabled | Independent layer switches; disabling BGM removes the music-file requirement. | Existing `--no-particles`, `--no-sfx`, `--no-bgm` and positive aliases; config file | Render / video |
| Watch execution: `execution_mode`, `parallel_watch_updates`, `watch_workers` | Interpreter, parallel, no worker cap | Execution is `interpreter` or `sono-gcc`; worker cap is null (machine default) or 1..=256. Sono-GCC retains interpreter fallback for unsupported paths. | Existing `--vm`, `--single-threaded`, `--watch-workers`; config file | Render / video |
| Sono-GCC preparation: `compile_sono_gcc_on_demand` | Enabled for generic/CLI config; disabled on GUI first-run because GUI starts background preparation | When disabled, an export does not synchronously compile on a cache miss; unsupported regions still use Watch VM. | Config file; current `compile-engine` command remains | Render / video when Sono-GCC is selected |
| Compiler cache | Persistent exact-graph cache enabled automatically on Windows | Cache identity and storage location remain automatic. Clearing is safe; eligible code is rebuilt later. | `clear-gcc-cache` | Render / video, Compiler cache |
| Gameplay UI: `ui` | Custom mode, disabled | The config contains the existing complete `RendererUiConfig`: profile name, elements, variables, fonts, scoring, assets, events, providers, and theme. UI mode is `custom`, `legacy`, or `off`. Profile assets/events resolve relative to the profile when selected or to the shared config file when authored inline. | Existing HUD flags plus `--ui-profile`; full object in config file | Gameplay UI tab; custom geometry and bindings remain in profile JSON |
| Watermark: `watermark` | Master enabled; moving mark disabled; attribution size 18 and opacity 0.72 | One master switch gates both marks. Turning it off forces the moving mark off. Fixed attribution text and bottom-left location are immutable; size, opacity, and margin may be configured. Moving text/image, font, size, scale, rotation, opacity, anchor, bounds, motion, timing, animation, layer, seed, and assets are configurable. | `--watermark-profile`, `--no-watermark`, `--moving-watermark`, `--no-moving-watermark`; full object in config file | Watermark tab; profile JSON supports all fields and image/font resources |
| Level options: `level_options` | Empty override list | Finite numeric overrides keyed by option index; engine defaults apply otherwise. | Existing repeatable `--level-option INDEX=VALUE`; config file | Engine options tab |
| Diagnostics: `trace_entity_id`, `profile`, `profile_frames`, `validate_determinism` | None / false / false / false | `profile_frames` only has an effect when profiling is enabled. The direct CLI `--profile-frames` flag requires `--profile`. Determinism validation performs additional work. | Existing trace/profile/determinism flags; config file | Render / video, Validation |
| Legacy compatibility: `concurrent_sfx_prepass` | false | Retained in serialized configs for compatibility; SFX is collected in the rendering Watch execution. Existing legacy CLI flags remain accepted with their current semantics. | Existing flags; config file | Serialized but not shown as a normal control |
| Scheduling pipeline: `frame_pipeline` | true | Overlaps Watch preparation with ordered render/output work. Disable for sequential A/B comparison. | Existing `--sequential-frames`; config file | Render / video |
| Notifications: `notifications` | Global and native enabled; native completion/failure only; Discord disabled; progress disabled | Bounded asynchronous event queue. Configure backend and destination event filters, per-event templates, progress milestones/interval, Discord guild/channel IDs, and a bot-token environment-variable name. Malformed notification-only settings are disabled with a diagnostic rather than invalidating the render config. | Shared config via `render-config`, or `render-video --notifications-config`; `--no-notifications` disables all | Notifications tab |
| Render jobs: `job_manager.max_concurrent_jobs`, `job_manager.default_priority` | 1, `normal` | Concurrent render limit is 1..=8; the default uses one renderer at a time. Priority aging prevents indefinite low-priority starvation. Queue storage is persistent and shared by the GUI and CLI. | `jobs` command; settings in the shared config | Job manager tab |

## Profile flags and watermark behavior

The existing positional `render-video` form is preserved. New profile flags are
additive:

```text
renderer render-video engine.zip skin.scp level.json.gz music.mp3 output.mp4 --ui-profile hud.json --watermark-profile watermark.json
renderer render-video engine.zip skin.scp level.json.gz music.mp3 output.mp4 --watermark-profile watermark.json --no-moving-watermark
renderer render-video engine.zip skin.scp level.json.gz music.mp3 output.mp4 --no-watermark
```

`--no-watermark` disables fixed attribution and the moving watermark together.
`--no-moving-watermark` leaves fixed attribution enabled. The API/config's
`watermark.enabled` master switch has the same meaning. The exact fixed
attribution rendered is &#x203B;Made with&#x300C;Sono-Renderer&#x300D;by Nexint.

## Preserved CLI and GUI surface

The existing CLI subcommands remain present: `ffmpeg-info`, `validate-project`,
`inspect-engine`, `compile-engine`, `inspect-level`, `inspect-resources`,
`compatibility`, `evaluate-node`, `run-watch`, `render-display-list`,
`render-video`, and `gui`. `render-config`, `validate-config`, `default-config`,
`clear-gcc-cache`, and `jobs` are additions. Existing `run-watch` tracing, VM, worker,
accounting, level-option, dimensions, layer, display-list, and diagnostic
options are unchanged.

The existing desktop Project / inputs, Engine options, and Render / video tabs
remain. The Gameplay UI, Watermark, and Notifications tabs, plus config
load/save controls, and Job manager tab are additions. The UI-profile tab selects a renderer-owned
profile; it is not an editor for arbitrary profile element trees.

## Current limitations

The encoder currently exposes only MP4/H.264 output and fixed audio settings;
H.264 CRF and speed/compression preset are configurable. WGPU uses the existing
adapter selection and does not expose device selection. The Sono-GCC cache
directory remains automatic and the cache cannot be disabled; users can clear
it through the new management action. UI profile
JSON is the source of truth for element-level layout, expressions, assets, and
fonts; the desktop UI does not duplicate that profile editor. The fixed
attribution text and location cannot be overridden.
