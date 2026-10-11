# Getting Started

Sono-Renderer brings three workflows into one application. The welcome screen
starts with the task you want to do; you do not need to configure the advanced
renderer before using any of them.

## Choose a workflow

The language selector at the top of Home also appears in Settings. Changing
it updates application-interface text immediately and is saved with workspace
preferences. This setting does not change chart data, project media, subtitles,
render configuration, or exported creative content. Initial non-English
catalog text is machine-drafted and still needs human review; untranslated
controls may remain in English.

- **Render a Level** opens Quick Render. Choose an engine, level, resources,
  optional music, and output, then check inputs and queue the MP4. A render
  project, profile, Chart Studio, and Presentation Studio are not required.
- **Create / Edit a Chart** opens Chart Studio. Open a LevelData file or choose
  **New LevelData**, edit its entities, optionally inspect a selected engine
  for archetype names, then validate and export `level.json.gz`.
- **Edit a Video** opens Presentation Studio. Start a composition and import
  ordinary video, audio, and image files; add text/effects, edit the timeline,
  record voiceover if an input device is available, and export. This works
  without Sonolus content, an engine, LevelData, or Chart Studio.
- **Open a Project** offers the existing render-project picker or the video
  composition opener. Render projects (`.sonoproject`) store render inputs and
  defaults. Video compositions (`.sonocomp`) store NLE composition state and
  media references. They are separate project types.
- **Learn** opens searchable, offline help for the current workflows,
  including vertical Shorts, URL media acquisition, and voiceover recording.

The home screen is keyboard-scrollable in a smaller window. Choose **Skip for
now** to go to Quick Render without completing a tour. Return to Home from the
top navigation or **More → Home / Getting Started**. Entering a render, chart,
video, project, or tour workflow dismisses first-run guidance. **Learn** opens
over the home screen so you can read first and then choose a workflow. Closing
the application before making a choice leaves first-run state incomplete, so
the home screen returns next time.

The welcome screen does not initialize Watch, Sono-GCC compilation, a video
decoder, or microphone capture. Device enumeration and media probing begin
only after entering the corresponding workflow; microphone capture still
requires an explicit meter or recording action. There is no timed Welcome
reappearance rule. Existing users who completed the previous onboarding keep
that choice through the local preference migration.

## Quick Render

1. Select the Sonolus engine ZIP, LevelData/chart, and engine resource package.
   These three inputs are required.
2. Select Music / BGM while **Include music** is enabled. Turn it off to render
   without music. MV and separate particle, effect, or background packages are
   optional.
3. Choose an output MP4 path and review the resolution, frame rate, and render
   range.
4. Choose **Check inputs**. It checks the inputs/output, looks for the existing
   FFmpeg pair without downloading anything, and runs the shared static
   compatibility analysis.
5. Review any findings and choose **Queue render**. Follow the job progress;
   when complete, use **Open Video** or **Open Folder**. Existing output files
   are not silently overwritten.

The Windows application provisions its pinned FFmpeg pair when export needs it
and no validated copy is installed. First-time setup then requires network
access. See [Installation](INSTALLATION.md) for packaged-app details.

You can drop supported render inputs or a directory into the app. Unique roles
are filled automatically; ambiguous engine/level/resource matches require an
explicit choice. Directory discovery is bounded. A dropped `.sonoproject`
opens the render project, and a dropped `.sonocomp` opens Presentation Studio.
This build does not register an operating-system file association or accept a
project path as a bare GUI command-line argument.

## Chart Studio

Chart Studio edits generic Sonolus LevelData. It can create a blank document,
open standalone JSON/gzip or supported legacy package forms, inspect the
selected engine's declared archetype names, preview a still frame through the
production Watch renderer, validate structure, and export gzip LevelData.
Engine-specific friendly labels/property controls require an exact
content-matched authoring schema. The editor does not infer gameplay meaning
from entity names. See [Chart Studio](CHART_STUDIO.md) for the complete model
and limitations.

## Presentation Studio

Presentation Studio is a standalone nonlinear video editor using ordinary
local media. Create a composition, import video/audio/stills into its media
bin, place clips on multiple tracks, trim/split/move, animate supported
properties, apply effects/masks/transitions, add titles, mix audio, preview,
save a `.sonocomp`, and export an MP4. The same composition supports landscape
and vertical dimensions such as 1080×1920. Microphone voiceover becomes an
ordinary WAV media source and timeline clip. Engine loading, Watch, LevelData,
and Sono-GCC are not part of this workflow.

URL acquisition is an optional source workflow exposed under **Project / inputs**
and Chart Studio in the **Acquire media from URL** section. It inspects and
explicitly downloads one supported HTTP(S) item to a local file. That file can
then be imported by Presentation Studio; URL tools are not required for local
media. See
[URL media acquisition](MEDIA_ACQUISITION.md).

The editor's practical workflow and current boundaries are documented in
[Presentation Studio](PRESENTATION_STUDIO.md), including [local subtitles and
caption editing](PRESENTATION_STUDIO_SUBTITLES.md). Voiceover device behavior
and validation limits are in [Microphone recording](PRESENTATION_STUDIO_AUDIO_INPUT.md).

## Projects and profiles

A render project stores reusable Sonolus input references and project-specific
render defaults. A profile stores reusable renderer settings. Both are
optional for direct Quick Render, and each queued job captures its effective
configuration. Presentation Studio compositions use their own `.sonocomp`
format and do not require a render project or profile.

The render-project **Recent projects** list is reused on the Welcome screen.
It does not currently include a separate recent-composition catalog; open a
video composition from Presentation Studio or drop its `.sonocomp` file into
the application.

## Help and replay

Use **More → Help and workflows** for searchable guidance and direct links to
Quick Render, Chart Studio, Presentation Studio, URL acquisition, and the
optional renderer workflow tour. **Replay Getting Started** deliberately
reopens first-run guidance. **More → Home / Getting Started** returns to the
goal-based home without resetting completion state.

## Other application areas

The top navigation keeps the common tasks visible: Home, Render a Level,
Chart Studio, Edit Video, and Jobs & History. **More** contains Tools, Settings,
and Diagnostics. Tools includes cache management, performance and regression
tools, compatibility analysis, and Program Preview. Settings contains saved
configuration/profiles, render defaults, gameplay UI, watermark, and
notifications. Diagnostics holds detailed runtime status and logs. The
Presentation Studio **Layout** menu changes panel visibility and timeline
scale; it does not change the active workflow or project. See
[workflows, layouts, commands, and shortcuts](WORKSPACE_COMMANDS.md).

## CLI

The GUI and CLI use the same renderer configuration and engine/runtime. A
direct command can render without creating a project or opening the GUI:

```powershell
$renderer = Join-Path $env:LOCALAPPDATA 'Programs\Sono-Renderer\Sono-Renderer.exe'
& $renderer render-video 'engine.zip' 'resources.scp' 'level.json.gz' 'music-file.ogg' 'output.mp4' --start-time 0 --duration 10 --fps 60 --width 1920 --height 1080
& $renderer render-video --help
```

For a portable copy, set `$renderer` to the path of `Sono-Renderer.exe` in its
extracted folder. Command-line rendering does not require desktop onboarding
or a project file.
