# Sono-Renderer Follow-up #41 — UI / Information Architecture

## Scope and evidence

This pass reorganizes the existing desktop UI around user workflows. It does
not add a creative domain or replace renderer, chart, composition, project,
queue, history, configuration, command, or persistence backends. The inspected
baseline was the current post-#40 working tree plus
`docs/ARCHITECTURE.md`, the #36 feature audit, #38 command/workspace report,
#39 release certification, #40 gap-closure report, #40.5 stabilization report,
and current workflow documentation. The repository was already dirty before
this pass; the development package manifest is correspondingly marked dirty.

Evidence labels used below:

- **Build-checked:** Cargo compilation completed.
- **Logic-tested:** headless tests exercised the route/persistence/dispatch
  behavior.
- **Packaged-artifact tested:** the Windows portable package was extracted
  and its non-GUI smoke workflow passed.
- **Manually tested:** a person used the actual application window. No such
  session was available in this environment.

## 1. Pre-overhaul UI inventory

At the start of this pass, `Desktop` in `src/gui.rs` exposed a large set of
renderer, settings, tools, Chart Studio, Presentation Studio, queue, and
history sections through one tab-indexed application page. Render-project and
profile controls, renderer status, and render actions were easy to encounter
alongside unrelated chart/video editing controls. The older workspace choice
also selected both a product area and a panel arrangement, so “workspace” did
not identify one consistent concept.

The same `Desktop` already owned the existing eframe window and shared
services. Chart Studio, Presentation Studio, queue/history, tools, settings,
onboarding, diagnostics, command palette, and shortcut editor were views in
that application, not separate executables. Presentation Studio already had
its own composition model and editor controls. The menu bar exposed repeated
Tools/Help entry points, including a global tools menu and a More menu.

The Home/Welcome goal cards and recent render-project list already existed.
Recent compositions did not have their own catalog. UI tests covered parts of
onboarding, file-drop routing, commands, and workspaces, but there was no
interactive desktop session or packaged-GUI test evidence in the #39/#40.5
reports.

## 2. Current workflow hierarchy

The persisted `WorkflowId` now selects the current task surface. `Desktop::ui`
renders the content for that route rather than showing the old all-sections
page. Common routes are visible in the top row; specialized application areas
are under **More**.

| Workflow | Current surface and boundary |
|---|---|
| Home / Welcome | User goals, Open Project choices, recent render projects, Learn/Help. No renderer/compiler/runtime details are presented as primary actions. |
| Render a Level | Quick Render keeps direct engine, level, resource, music, output, input checking, and queue submission. Project/profile management is collapsed. |
| Chart Studio | Chart authoring, selected-engine schema/archetype inspection, Watch preview, validation/export, audio and tempo analysis. Tempo analysis remains the same Chart Studio analyzer. |
| Edit Video | Presentation Studio's standalone composition editor. Ordinary media remains usable without Sonolus. Layout selection is separate from the project route. |
| Jobs & History | Queue and durable render history are adjacent pages in the existing production area. |
| Tools | Cache, Performance Lab, Regression Lab, compatibility analysis, and Program Preview. |
| Settings | Saved render configuration/profiles, engine options, render defaults, gameplay UI, watermark, and notifications. Render-input paths and project details live in Quick Render. |
| Diagnostics | Detailed runtime status and renderer logs are shown in a dedicated route. |

Quick Render remains a focused path and uses the same `RenderConfig`, shared
compatibility analyzer, job manager, and renderer. The collapsed project area
keeps recent/open/save project controls, project validation, project profile
selection, and project-job creation reachable without repeating the required
input editor in Settings.

Chart Studio and Presentation Studio remain different work purposes. The
Presentation route continues to use `CompositionProject`, the existing
composition preview/export graph, and its own document editing/undo path. It
does not load Watch because the route exists; gameplay is still an optional
composition source.

## 3. Navigation, menus, and contextual tools

The top navigation presents **Home**, **Render a Level**, **Chart Studio**,
**Edit Video**, and **Jobs & History**. **More** provides **Tools**,
**Settings**, **Diagnostics**, the contextual “Open a tool” submenu, Home,
Help, the Command Palette, and Keyboard Shortcuts. The duplicate second-row
global Tools and Help menus were removed. File operations vary by route;
opening a video composition from Home/other application areas first enters
Presentation Studio and then invokes the existing composition-open command.
Presentation-only Edit, Timeline, and Layout menus appear only in Presentation
Studio.

Production, Tools, and Settings page selectors now dispatch registered
navigation commands. Those stable command IDs also appear in the command
search/shortcut registry. Settings search only searches Settings sections;
cache, history, diagnostics, and chart pages are reached through their own
workflow navigation. Tool shortcuts lead into the existing tool or source
workflow rather than creating another implementation. For example, Tempo
Analysis goes to Chart Studio, which owns the current music selection and
tempo state.

The File menu and Home actions preserve the existing command paths for render
projects and compositions. Editing operations still use the existing document
commands/undo implementations. Lower-level continuous widgets (sliders,
text fields, input pickers) remain direct controls where the established
architecture does not model each adjustment as a discrete command.

## 4. Workflows versus layouts

`WorkflowId` is task/navigation state. Existing `WorkspaceId` remains the
persisted panel-layout identity, preserving the #38 settings format and
custom layout support. New preferences default to Home plus the Editing
Presentation layout. The Presentation Studio Layout menu exposes Editing and
Voiceover panel arrangements; saved layouts still store visibility, timeline
zoom, and track height. Layout changes do not change the workflow, project,
chart, or composition.

Workspace-preferences schema is now version 2. Schema 1 migration maps its
former active workspace to the corresponding workflow while preserving its
layout. Old Quick Render, Charting, and Advanced/Diagnostics workspace IDs
remain readable for migration and saved-layout compatibility; they are not
offered as Presentation Studio layout presets. Existing atomic persistence,
bounds validation, shortcut overrides, future-schema rejection, and
corruption fallback remain in place.

Switching routes leaves open render inputs, the chart document, composition,
and the active Presentation layout in memory. The route change does not close
documents. Replacing a document still passes through its existing safety
checks. This pass does not add multi-window or multi-document infrastructure.

## 5. Home, recents, onboarding, and empty states

Home continues to offer the task-oriented Render a Level, Create/Edit a Chart,
Edit a Video, Open a Project, and Learn actions. Established users can return
from the main Home route or **More → Home / Getting Started**. **More → Help
and workflows → Replay Getting Started** replays first-run guidance. Completion
and Skip persistence are unchanged; the UI overhaul does not reset onboarding
or add a timer-based nag rule.

Recent work reuses the existing render-project recent list. There is no second
recent-composition database; Home and File can open a `.sonocomp`, and a
dropped `.sonocomp` routes into Presentation Studio. Existing direct project
drop/open behavior remains. OS file associations and bare project-path GUI
arguments remain unimplemented as documented before this pass.

Empty Chart Studio and Presentation Studio documents remain their existing
authoritative document defaults. Empty Recent Projects explains that only
render projects are currently listed. No new backend objects or tutorial-only
projects were added.

## 6. Status, settings, diagnostics, and lifecycle

Queue/progress status remains a shared renderer/job concern and is visible on
active render surfaces; the queue/history route is the production status
browser. Deep renderer logs and Sono-GCC status are restricted to Diagnostics
rather than shown on ordinary Home or video-editing surfaces. Settings search
is limited to its own sections. Project inputs and project overrides are kept
with Quick Render's collapsed project controls, not treated as application
preferences.

No backend service lifecycle was rewritten. Inspection of startup and view
entry paths found that Home does not start Watch evaluation, Sono-GCC
compilation, video decoding, microphone capture/device enumeration, speech
transcription, or BPM analysis. Existing application-wide config/profile,
queue, history, and preference stores are still opened during normal desktop
startup; the queue manager is shared infrastructure, not a second UI-specific
queue. Chart/media/runtime work starts through its existing owning workflow.
This is a source-inspection finding, not a measured cold-start performance
claim.

The active UI remains a single eframe window with routed central content and
scrollable workflow surfaces. Floating/dockable panels, independent editor
windows, and a recent-composition catalog remain outside the current
implementation.

## 7. Documentation and packaging

Updated:

- `docs/WORKSPACE_COMMANDS.md` — workflow/layout distinction, navigation,
  commands, persistence, and current limits.
- `docs/GETTING_STARTED.md` — actual Home/More navigation and current product
  areas.
- `docs/ARCHITECTURE.md` — the `WorkflowId` / `WorkspaceId` ownership split.
- `README.md` — links to the user guide and this report.
- `scripts/package-windows.ps1` — includes `WORKSPACE_COMMANDS.md` with the
  packaged user documentation.

The development portable package was rebuilt from the final release-mode
binary under `target/ui41-final-package/distribution`. It contains the updated
Getting Started and workflow/command guide. SHA-256 of
`Sono-Renderer-0.1.0-windows-x86_64-portable.zip`:

```text
f60b9426090153cb278d4b7d201615db39995bf6673619d5d72d059399aaf19b
```

The archive manifest says `dirty: true` and `reproducible_build: false`; this
is development evidence, not a release candidate. `rc.exe` was unavailable,
so Windows version-resource metadata was omitted. Inno Setup `ISCC.exe` was
unavailable, so no installer was produced. The existing portable smoke script
passed extraction under Unicode/spaced paths, foreign working directory,
restricted PATH, `--version`, `--help`, jobs, and media-tool-info checks. This
was a CLI/package-layout smoke test, not a GUI launch or clean-machine test.

## 8. Validation results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | **Pass**; formatter emitted a path canonicalization warning for `C:\Users\Admin`. |
| `cargo check --locked --all-targets` | **Pass**. |
| Workspace preference/command tests | **Pass: 7 tests**, including schema-1 migration, future-schema rejection, command search, layout retention, and atomic round-trip. |
| GUI logic tests | **Pass: 35 tests**, including first-run skip, dropped composition routing, route/layout independence, preservation of open dirty composition and renderer inputs, route command dispatch, Settings search, and contextual command enablement. |
| `cargo build --locked --release --bin renderer` | **Pass**; warning: `rc.exe` unavailable, Windows version resource omitted. |
| CLI help from release executable | **Pass** in the package smoke test. Existing CLI commands and renderer backends were not structurally changed. |
| Windows portable package | **Packaged-artifact tested for non-GUI layout/CLI only**; smoke passed. Installer unavailable because ISCC was not installed. |
| GUI screens, keyboard/mouse use, DPI/window sizing, OS file association | **Not interactively tested** in this environment. |

### Performance evidence

No separate GUI timing instrumentation was added. Cold desktop startup, Home
first paint, opening each workflow, workflow-switch latency, command-palette
search latency, and workspace restore time were **not measured**. The previous
#39 report's 91.17 ms median is explicitly a CLI `--version` process-start
sample and excludes GUI initialization; it is not reported as a UI benchmark.
The release build's elapsed time is a build measurement, not application
performance.

## 9. End-to-end workflow evidence and limits

| Scenario | Evidence in this pass |
|---|---|
| Home → Quick Render / Chart / Presentation / Production | Route dispatch/build-checked; workflow command and preservation tests passed. No UI clicks or render performed through the actual desktop window. |
| Open standalone composition without Sonolus | Existing dropped-composition GUI logic test passes, including empty sources and Presentation route. No interactive media edit/export in this pass. |
| Chart authoring/export and Watch | Existing #39/#40.5 backend and fixture evidence remains the source; this UI-only pass did not repeat the full chart author/export/render workflow. |
| Jobs/history/tools/diagnostics | Dedicated routed views and registered commands build-check; no live interactive browsing performed. |
| Restart/persistence | Schema migration and save/load tests pass. A manual GUI restart/layout editing session was unavailable. |
| Packaged application | Portable package and restricted-PATH CLI smoke pass. No packaged GUI interaction, installer run, clean machine, or OS association test. |
| Sonolus / VM / GCC semantics, rendering, audio, watermark | No backend code changed. The #39/#40.5 evidence was not rerun as part of this UI pass; this report makes no new semantic-regression claim. |

## 10. Remaining UI debt

- `Desktop`/`src/gui.rs` remains a large frontend module. This pass separated
  route ownership and user-visible workflow content without mechanically
  splitting the God object or rewriting backend ownership.
- The UI is one native window and does not provide floating docks or multiple
  open editor windows.
- Recent projects cover render projects, not `.sonocomp` compositions.
- Actual GUI interaction, accessibility-reader behavior, high-DPI layouts,
  and installed first-run are still unverified.
- Cold-start and route performance have no dedicated measurements.
- Release readiness remains **NOT READY FOR RELEASE CANDIDATE** as stated by
  the current #39 certification: dirty source identity, missing release tools
  and metadata, incomplete publication review, and clean-machine installed
  GUI certification remain outside this IA pass.

## Result

**Implemented and build-checked:** the main app now routes into distinct user
workflows; Quick Render remains focused; Settings no longer duplicates render
input editing; tool and settings pages use registered navigation commands;
Presentation layouts are separate from route state; docs and portable package
contents describe the new navigation.

**Logic-tested:** route persistence/migration, direct composition drop,
skip-onboarding, route/layout separation, open-document retention, command
dispatch, and settings search.

**Not visually or manually verified:** no GUI screenshot or interactive
session is claimed. The environment supported Windows builds and portable
CLI package smoke testing, but not visual inspection of the live desktop UI.
