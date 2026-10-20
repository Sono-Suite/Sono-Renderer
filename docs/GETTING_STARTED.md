# Getting Started

Sono-Renderer's default desktop interface focuses on rendering Sonolus charts.
The welcome screen takes you to Quick Render, where you load chart inputs,
configure output, and export video. Program Preview can render still frames
through the production renderer.

## Home and first run

The language selector at the top of Home also appears in Settings. Changing it
updates application-interface text immediately and is saved with workspace
preferences. This setting does not change chart data, project media, render
configuration, or exported content. Initial non-English catalog text is
machine-drafted and still needs human review; untranslated controls may remain
in English.

Home provides **Render a Level**, **Open a Project**, and **Learn**. Choose
**Skip for now** to open Quick Render without completing the render tour. Return
Home from the top navigation or **More / Home / Getting Started**. Learn opens
over Home with searchable, offline guidance. Closing the app before making a
choice leaves first-run state incomplete, so Home returns next time.

The welcome screen does not initialize Watch or Sono-GCC compilation. There is
no timed Welcome reappearance rule. Existing users who completed the previous
onboarding keep that choice through the local preference migration. If a saved
workspace points to a retained editor route, the app opens Quick Render.

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
5. Review any findings and choose **Queue render**. Follow job progress; when
   complete, use **Open Video** or **Open Folder**. Existing output files are
   not silently overwritten.

The engine loader reads ZIP packages containing `EngineWatchData`. Levels can
be supplied as standalone JSON, gzip-compressed JSON, or a legacy level ZIP
package. Resource packages use SCP/ZIP. The renderer consumes Watch data; it
does not run an interactive Sonolus client session.

The Windows application provisions its pinned FFmpeg pair when export needs it
and no validated copy is installed. First-time setup then requires network
access. See [Installation](INSTALLATION.md) for packaged-app details.

You can drop supported render inputs or a directory into the app. Unique roles
are filled automatically; ambiguous engine/level/resource matches require an
explicit choice. Directory discovery is bounded. A dropped `.sonoproject`
opens the render project. `.sonocomp` files remain part of the retained source
infrastructure but do not open in the default UI. This build does not register
an operating-system file association or accept a project path as a bare GUI
command-line argument.

## Retained chart authoring infrastructure

Chart Studio has no route in the default desktop interface. The guide below is
an implementation reference for builds that register the retained workflow.

The Chart Studio implementation still edits generic Sonolus LevelData. Its
document model, authoring schema, preview, validation, and export paths remain
in source, but its workflow is not registered in the default interface. See
[Chart Studio](CHART_STUDIO.md) for its model and limitations.

## Retained video editing infrastructure

Presentation Studio has no route in the default desktop interface. Its guide
and sub-guides describe retained source infrastructure, not an available
default editing workflow.

Presentation Studio's UI, CompositionProject model, timeline, media, preview,
audio, and export implementation remain in source. Its workflow and document
openers are not registered in the default interface. Composition documents use
the existing `.sonocomp` format; no project or serialization migration is
performed. See [Presentation Studio](PRESENTATION_STUDIO.md) and
[Microphone recording](PRESENTATION_STUDIO_AUDIO_INPUT.md) for implementation
details.

## URL acquisition, projects, and profiles

URL acquisition is available under **Project / inputs**. It inspects and
explicitly downloads one supported HTTP(S) item to a local file. See
[URL media acquisition](MEDIA_ACQUISITION.md).

A render project stores reusable Sonolus input references and project-specific
render defaults. A profile stores reusable renderer settings. Both are optional
for direct Quick Render, and each queued job captures its effective
configuration. Existing `.sonocomp` documents and composition services remain
in source for future UI registration. The Welcome screen's **Recent projects**
list only shows render projects.

## Compatibility scope

The renderer targets Sonolus Watch-mode chart rendering. **Program Preview**
renders still frames through that same path; it is not Sonolus Preview mode.
The three Sonolus 1.1.5 Watch memory-length fields are implemented with legacy
defaults, but this does not establish full 1.1.5 or engine-wide compatibility.
Some Watch operations remain approximate, and the new fields' out-of-range
behavior is inferred from static prototype analysis rather than confirmed by
running the Sonolus APK. See [Compatibility Analyzer](COMPATIBILITY_ANALYZER.md)
and [Current limitations](KNOWN_LIMITATIONS.md) before relying on unfamiliar
content.

## Help and other application areas

Use **More / Help and workflows** for searchable guidance and direct links to
Quick Render, URL acquisition, and the optional renderer workflow tour.
**Replay Getting Started** deliberately reopens first-run guidance.

The top navigation keeps Home, Render a Level, and Jobs & History visible.
**More** contains Tools, Settings, and Diagnostics. Tools includes cache
management, performance and regression tools, compatibility analysis, and
Program Preview. Settings contains saved configuration/profiles, render
defaults, gameplay UI, watermark, and notifications. Diagnostics holds
renderer status and logs. Chart Studio and Presentation Studio routes remain
in source but are omitted from default navigation and action registration. See
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
