# Workflows, layouts, commands, and shortcuts

Sono-Renderer's default desktop UI focuses on loading Sonolus render inputs,
configuring renders, previewing supported output, and exporting video. Chart
Studio and Presentation Studio implementations remain in the source, but their
workflow routes and feature commands are omitted from default navigation,
command search, and shortcut assignment.

## Default navigation

The main toolbar presents **Home**, **Render a Level**, and **Jobs & History**.
**More** opens **Tools**, **Settings**, and **Diagnostics**, plus Getting
Started, Help, the Command Palette, and Keyboard Shortcuts. The File menu
exposes render-project operations. A remembered Chart Studio or Presentation
Studio workflow falls back to Quick Render when the app starts.

| Surface | Primary UI and ownership |
|---|---|
| Home | Open Quick Render or a render project, see recent render projects, and open Learn/Help. |
| Render a Level | Engine, LevelData, resources, music, output, input check, preview support, and queue submission. Project/profile controls are optional. |
| Jobs & History | Queue state, active jobs, outputs, prior attempts, diagnostics, performance, and Render Again. |
| Tools | Cache manager, Performance Lab, Regression Lab, compatibility analysis, and Program Preview. |
| Settings | Saved render configuration/profiles, engine options, render defaults, gameplay UI, watermark, and notifications. |
| Diagnostics | Detailed renderer status and logs, separated from the normal rendering surface. |
| Retained feature routes | Chart Studio and Presentation Studio remain implemented but are not registered in the default UI. Their models, serialization, and rendering/editing helpers are preserved. |

## Command registry

The complete command registry lives in `src/workspace.rs`. Stable command IDs,
descriptions, categories, keywords, default bindings, and document contexts
remain available to internal feature code and compatibility readers. A separate
default UI command iterator filters out chart-authoring and composition editing
commands from the palette, shortcut editor, and shortcut handling. Re-registering
an optional feature in a future interface can reuse the existing commands and
dispatch paths.

Open the **Command Palette** from **More / Command Palette** or press
**Ctrl+Shift+P** (Command on macOS). It searches commands exposed in the default
UI. The **Keyboard Shortcuts** editor uses that same subset, supports one user
binding per visible command, checks conflicts against saved bindings, and can
restore defaults. Hidden feature shortcut records remain readable in the
existing preferences file but are not activated by the default interface.

The default File menu opens and saves render projects (`.sonoproject`). The
composition format (`.sonocomp`), Chart Studio LevelData model, editing commands,
and serialization paths remain in source. Composition drops do not route into
the hidden editor from the default interface.

## Settings and persistence

Workspace preferences are stored atomically in the versioned
`workspace_preferences.json` under the application configuration root (or the
`SONO_RENDERER_WORKSPACE_PREFERENCES` override). Schema 2 adds a separately
persisted workflow route. Schema 1 migrates the old active workspace into a
workflow and retains the old layout selection. Invalid/corrupt preferences
fall back to defaults with an in-app warning; unsupported future schemas are
reported rather than overwritten silently.

The preference file stores workflow selection, panel/layout state, custom
layouts, and shortcut overrides. Its schema and feature identifiers are
unchanged. Existing project, chart snapshot, and composition data are not
rewritten by removing the default UI routes. Existing editor routes are mapped
to Quick Render at application startup; their documents and saved layouts remain
available to code that registers those features.

The current UI uses distinct routed surfaces within one eframe application
window. It does not implement floating docks, multiple independent editor
windows, a layout designer, or a plugin API. Workflow and command registration
provide a small seam for future interfaces to opt into retained features.