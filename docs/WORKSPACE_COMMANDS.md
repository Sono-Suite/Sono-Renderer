# Workflows, layouts, commands, and shortcuts

Sono-Renderer groups the desktop UI by the user's task. A **workflow** selects
the active product surface; a **layout** stores panel visibility and timeline
dimensions inside Presentation Studio. Switching workflows does not change a
layout, close a document, or replace a project.

## Main navigation

The main navigation presents **Home**, **Render a Level**, **Chart Studio**,
**Edit Video**, and **Jobs & History**. **More** opens **Tools**, **Settings**,
and **Diagnostics**, plus a contextual tool list, Getting Started, Help, the
Command Palette, and Keyboard Shortcuts. The File menu changes with the active
workflow. Opening a different workflow keeps the current render project, chart
document, and video composition in memory; opening/replacing a document still
uses that document's existing safety checks.

| Workflow | Primary UI and ownership |
|---|---|
| Home | Choose a goal, open a render project or composition, see recent render projects, and open Learn/Help. |
| Render a Level | Direct engine, LevelData, resources, music, output, input check, and queue submission. Project/profile controls are collapsed. |
| Chart Studio | LevelData authoring, engine archetypes, audio/tempo analysis, Watch preview, validation, and export. |
| Presentation Studio | Standalone media bin, source/program views, timeline, inspector, effects, captions, voiceover, and export. Sonolus is optional. |
| Jobs & History | Queue state, active jobs, outputs, prior attempts, diagnostics, performance, and Render Again. |
| Tools | Cache manager, Performance Lab, Regression Lab, compatibility analysis, and Program Preview. Tempo analysis remains in Chart Studio and uses the same analyzer. |
| Settings | Saved render configuration/profiles, engine options, render defaults, gameplay UI, watermark, and notifications. Project inputs remain in Quick Render. |
| Diagnostics | Detailed renderer status and logs, separated from ordinary editing and rendering surfaces. |

Presentation Studio's **Layout** menu switches the built-in **Editing** and
**Voiceover** panel arrangements, applies saved panel layouts, changes visible
panels/timeline zoom/track height, and resets the active layout. It does not
switch the workflow or document. Legacy Quick Render, Charting, and
Advanced/Diagnostics layout IDs are still decoded when migrating old settings;
they are not shown as Presentation Studio layout presets. Floating or
dockable panels are not implemented by the current eframe UI.

Recent-project support reuses the existing render-project catalog. It does not
create a second recent list for video compositions; open a composition through
Home/File or drop its `.sonocomp` into the window.

## Commands and keyboard shortcuts

The command registry lives in `src/workspace.rs`. Stable command IDs,
descriptions, categories, keywords, default bindings, and document contexts
feed the Command Palette, shortcut editor, menu items, and dispatch. Document
edits still go through the existing Chart Studio or Presentation Studio edit
and undo paths. Top-level workflows and the production, tools, and settings
subpages use registered navigation commands. Navigation commands set the
workflow independently of panel-layout preferences.

Open the **Command Palette** from **More → Command Palette** or press
**Ctrl+Shift+P** (Command on macOS). Search includes terms such as `split`,
`bpm`, `subtitle`, `voiceover`, `render`, and `history`. Disabled contextual
commands show why they cannot run. The **Keyboard Shortcuts** editor uses the
same registry, supports one user binding per command, checks conflicts, and
can restore defaults. Redo retains Ctrl+Y as an alternate until the user
customizes that command. Single-letter shortcuts require a modifier except
for existing editor-specific shortcuts handled by their established context.

The File menu exposes different project operations by workflow. Render
projects (`.sonoproject`) contain render inputs and defaults; compositions
(`.sonocomp`) contain standalone NLE state. Opening/acquiring media invokes
the registered project/media commands, and composition editing continues to
use the established document command and undo behavior.

## Settings and persistence

Workspace preferences are stored atomically in the versioned
`workspace_preferences.json` under the application configuration root (or the
`SONO_RENDERER_WORKSPACE_PREFERENCES` override). Schema 2 adds a separately
persisted workflow route. Schema 1 migrates the old active workspace into a
workflow and retains the old layout selection. Invalid/corrupt preferences
fall back to defaults with an in-app warning; unsupported future schemas are
reported rather than overwritten silently.

The preference file stores workflow selection, panel/layout state, custom
layouts, and shortcut overrides. It is UI state, not part of a render profile,
render project, video composition, or immutable render-job snapshot. The
settings search jumps to the existing settings page; it does not create a
second settings model.

The current UI uses distinct routed surfaces within one eframe application
window. It does not implement floating docks, multiple independent editor
windows, or a composition recent-project database.
