# Sono-Renderer Follow-up #38: Workspaces and Commands

**Scope:** application ergonomics, command discovery, keyboard shortcuts, and
recoverable workspace preferences. No rendering, chart, audio, or video
semantics were added.

## Baseline inspected

The #36 feature audit and #37 implementation report were read along with the
current GUI and architecture documentation. The desktop frontend is one large
`egui`/`eframe` application in `src/gui.rs`. It presents renderer settings as
tabs and Presentation Studio as a scrollable page of collapsible sections.
Before this change, many actions were handled in local widget callbacks and
Presentation Studio had its own hard-coded shortcut handling. There was no
general command registry or dock manager. A settings search already existed.

The current frontend does **not** support draggable or floating panels. This
implementation keeps that boundary explicit rather than exposing fake docking
controls.

## Command architecture

`src/workspace.rs` now owns stable `CommandId` values and one static
`COMMANDS` registry. Each command has an ID, name, category, description,
search terms, default binding, and context requirement. `Desktop::dispatch_command`
in `src/gui.rs` is the shared action path used by menus, the toolbar, keyboard
dispatch, and the command palette. Existing document mutations still call the
existing project/undo operations; command dispatch is not a replacement undo
system.

The registered command families are:

- Application and navigation: palette, shortcut settings, Quick Render,
  Chart Studio, Video Editing, Voiceover, Advanced / Diagnostics, reset.
- Render-input projects: new/open/save/save-as. These remain distinct from
  Presentation Studio composition commands.
- Presentation: new/open/save/save-as/export, play/pause, frame stepping,
  voiceover record/stop, and subtitle generation.
- Quick Render: URL media acquisition through the existing render-input
  assignment flow.
- Timeline: copy/paste/duplicate/delete/split/ripple-delete, snapping, and
  Add Keyframe.
- Chart and renderer: tempo analysis, LevelData export, and MP4 render.

Stable IDs include `timeline.split`, `timeline.ripple_delete`,
`timeline.add_keyframe`, `chart.analyze_tempo`, `chart.export_level`,
`project.new`, `project.open`, `project.save`, `project.save_as`,
`presentation.generate_subtitles`, `presentation.record_voiceover`,
`render.acquire_media_from_url`, and `render.start`.

Enablement is contextual and is shared by menus, palette, and shortcuts.
Checks include active workspace, selected clip, locked tracks, valid split
position/transition state, selected audio/model availability, active capture
or composition job, chart audio, and active render work. The global Render
button retains its existing availability across the main settings tabs.
Unavailable palette/menu actions explain the immediate reason.

## Palette, menus, and shortcuts

The application-wide palette is available from Help and the toolbar; its
default binding is Ctrl+Shift+P, displayed with the platform primary modifier
(Command on macOS). Search is a small in-process fuzzy/subsequence search over
registry metadata. It supports queries such as `split`, `bpm`, `subtitle`,
`voiceover`, `url`, and `render` without scanning projects or loading engines.
Rows show their current binding and disabled reason.

The Keyboard Shortcuts window uses the same registry. Users can search, bind,
clear, reset one binding, or restore all defaults. It detects collisions and
requires explicit replacement; replacement disables the prior command's
binding instead of leaving an ambiguous dispatch. One user binding per
command is supported. Redo retains its existing Ctrl+Y default alias until
the user customizes Redo. User-assigned bare letters are rejected; the
existing Split default remains selection/workspace-contextual.

Global dispatch ignores repeated key-down events, does not run while egui has
keyboard input, and is suppressed while shortcut capture/conflict, the command
palette, shortcut editor, Help/tour, or existing blocking confirmation
windows are active. A key used to record a shortcut is not also dispatched as
an application command in that frame. This protects text/path/numeric entry
and prevents commands from firing behind the app's confirmation windows.

The Workspaces, View, File, Edit, Timeline / Tools, and Help menus expose
registered actions. The main render/export button and key Presentation/Chart
controls route through the same dispatcher. `Acquire Media From URL` enters
Quick Render, opens the existing render-input acquisition section, and
focuses its URL field; it reuses the current importer and does not claim
Presentation Studio media-bin integration.

## Workspace behavior

Built-in workspaces are Quick Render, Charting, Video Editing, Voiceover, and
Advanced / Diagnostics. They navigate to existing screens. Presentation
Studio's View menu can show/hide its existing collapsible sections and persist
timeline zoom and track height. The Voiceover workspace starts from a
voiceover-oriented section layout. Each built-in workspace remembers its own
layout across workspace switches. Users can save up to 64 named custom
layouts; Reset Current Workspace restores the current built-in defaults.

This is layout customization supported by the current UI, not docking:
sections cannot be dragged into new regions, floated, or resized as arbitrary
panels. Chart Studio and the renderer settings do not yet have independently
resizable panel layouts. There are no floating window positions to recover
when monitor geometry or DPI changes.

Workspace and shortcut preferences live together in versioned
`workspace_preferences.json` under the existing application configuration
root. `SONO_RENDERER_WORKSPACE_PREFERENCES` can override its path for isolated
testing. Writes use a same-directory temporary file and atomic replacement.
Loading rejects unsupported future schemas, migrates the earlier active
layout snapshot into the per-workspace map, clamps layout dimensions, and
drops malformed/removed command bindings. Save-time validation bounds names,
layout dimensions, shortcut keys, and custom-layout count.

This data is user-interface state. It is not a render profile, project,
composition, or queued job snapshot, and changing it cannot change render
semantics. No command history or second undo stack was added. Existing
settings search was retained.

## Discoverability and documentation

Help's Getting Started topic now explains the command palette, shortcut
customization, workspace choices, View → Panels recovery, and Reset Current
Workspace. The README documentation index links to the new user guide and
this report. The guide points back to the registry instead of maintaining a
second command-name/shortcut table.

## Validation

- **Build-checked:** `cargo check` passed after the final code changes.
- **Unit/integration tests:** `cargo test --lib` completed with 400 passed,
  0 failed, and 8 ignored. This run exercised the full library test suite,
  including real renderer/pipeline comparisons. After that run, the Render
  toolbar was aligned with shared command enablement, project controls were
  routed through the registry, and a workspace schema-migration assertion was
  added. Final code passed `cargo check`, `cargo test --lib workspace`
  (9 passed), `cargo test --lib command` (8 passed), and
  `cargo test --lib new_render_project_button_action` (1 passed). The full
  suite was not rerun after those final changes.
- **Preference-specific coverage:** fuzzy search, shortcut conflict and
  replacement, unsafe bare-letter rejection, unknown command rejection,
  atomic-store round trip, missing-file defaults, old-layout migration,
  future-schema rejection, custom layouts, per-workspace retention, and reset.
- **GUI logic coverage:** shared workspace navigation/reset, layout retention,
  contextual disabled commands, valid split position, URL acquisition
  navigation, and render-project command dispatch.
- **Formatting:** `rustfmt --check src/workspace.rs` passes. Repository-wide
  `cargo fmt -- --check` remains failing because `src/gui.rs` and other large
  source files contain formatting differences, including this monolithic GUI
  file. Formatting the entire GUI would create extensive unrelated churn. No
  whole-file formatting rewrite was applied.
- **Not manually exercised:** desktop mouse/keyboard interaction, screen
  reader behavior, packaged application behavior, and macOS/Linux shortcut
  labels. The tests exercise GUI dispatch logic, not a live desktop session.

## Deferred / known limitations

- Real dock/float/drag-rearrange panel layouts and per-panel resizing require
  a docking-capable frontend architecture; this task does not add one.
- Context menus for every timeline/media/chart item remain absent; existing
  visible controls and shared menus remain the discoverable paths.
- Shortcut preset import/export and multiple user bindings per command are
  not implemented. Neither was needed to establish safe in-app rebinding.
- The desktop frontend retains its existing single active render-project,
  Chart Studio document, and Presentation Studio composition context. No
  multi-window/document routing layer was introduced.
- Recent-command history is intentionally omitted; it is not needed for
  command discovery and would add another persistent history concept.
- No installed-package or interactive GUI acceptance session was available.

## Evidence key

**Implemented / wired** means a production GUI path calls the shared
dispatcher. **Tested** means automated library tests passed. **Build-checked**
means `cargo check` passed. **Not manually tested** means no live desktop
interaction was performed. Docking is **not implemented** and is not
represented as complete.
