# Sono-Renderer Follow-up #33 — Welcome and onboarding

## Scope and baseline

This pass refreshed the application's existing Welcome surface and onboarding
state. It did not add another application, editor, render path, or project
format. The baseline focused onboarding tests passed before edits (2 passed).
The code review also read the current GUI, onboarding store, startup/drop
routing, `docs/ARCHITECTURE.md`, the #32.5/#32.6/#32.7/#32.8 reports, and the
current Chart Studio and Presentation Studio guides.

| Area | Before this pass | After this pass |
|---|---|---|
| Primary Welcome goals | Quick Render, render project, gameplay UI, Learn; Chart Studio was a lower separate card; standalone video was only mentioned in copy | Five goal-oriented destinations: Render a Level, Create / Edit a Chart, Edit a Video, Open a Project, Learn |
| Standalone editing discovery | The copy said ordinary video was supported, but there was no direct Welcome action | Direct Presentation Studio action explicitly says Sonolus content is unnecessary |
| Onboarding state | Local atomic schema 1 with a completion bool and gameplay-tour/first-render state | Local atomic schema 2 with explicit skipped/completed state and a welcome-content version; schema 1 migrates without re-showing completed Welcome |
| Project opening | Existing render-project picker/recents; composition opener inside Presentation Studio | Welcome distinguishes render projects and video compositions; existing render recents appear on Welcome |
| File drop | Render project detection ran after the Presentation Studio media-drop branch; `.sonocomp` did not open from Welcome | A single `.sonocomp` opens Presentation Studio; a render project dropped from any workspace routes to Project / inputs |
| Help | Small renderer-centric searchable topic list | Searchable guides cover current render, chart, standalone video, Shorts, local/URL media, voiceover, projects, export, and advanced renderer topics |
| User documentation | Getting Started and README denied current NLE capabilities; architecture guide said no persisted composition project | Getting Started, README, and architecture ownership/persistence map now match the current product |

## 1–5. Existing onboarding, persistence, issues, and root causes

**Verified by code inspection:** `Desktop::default` loads a local onboarding
preference file; missing preferences show Welcome. The prior state used
`welcome_completed`, a gameplay-presentation-tour flag, and first successful
render metadata. Saving was already same-directory staged/atomic. The GUI had
no elapsed-time or interval-based Welcome reappearance rule. Help already
offered Replay Welcome and Reset onboarding. Closing an incomplete first run
left it incomplete.

The old screen primarily presented the product as a Sonolus renderer and
foregrounded gameplay HUD configuration. Chart Studio was discoverable, and
the body did mention standalone video editing, but there was no prominent
Edit a Video action or clear statement that a video project needs no Sonolus
content. Help omitted URL acquisition, voiceover, vertical editing, and the
current continuous timeline workflow. The guide and README still described a
time-sorted still-preview workflow, and the architecture guide contradicted
its own composition section by saying no persisted NLE project existed.

The root cause was product expansion after the earlier first-run page: the
welcome hierarchy and help copy had not been brought forward with the NLE.
The persistence model also treated completion and skipping as the same
unexplained boolean and had no content-version marker.

## 6–12. Welcome behavior and state

**Implemented:** the Welcome home now presents the five user goals first:

- **Render a Level** opens the existing Quick Render workflow.
- **Create / Edit a Chart** opens the existing Chart Studio workspace.
- **Edit a Video** opens the existing Presentation Studio workspace and
  states that Sonolus content is not needed.
- **Open a Project** offers the existing render-project picker, existing
  video-composition picker, and up to five entries from the existing render
  project recent list.
- **Learn** opens searchable help and an optional Render Workflow Tour.

The page uses the existing egui workspace, scrolls at short window heights,
and does not add a separate beginner workflow. Choosing Render, Chart, Video,
Project, or the tour marks the welcome complete and enters that workflow.
**Learn** opens help over the Welcome so a new user can read first and then
choose. **Skip for now** records an explicit skipped disposition. **Help →
Replay Welcome** reopens it without clearing completion; **Reset onboarding**
deliberately clears the welcome and tour state. If the first-run screen is
closed before a workflow or skip choice, persisted state remains incomplete
and it returns at next launch.

The preference format is now schema 2. It records completion, explicit skip,
and `welcome_version_completed`; future optional What's New content can evolve
separately without resetting first-run guidance each release. Schema 1
completed state migrates as completed, so upgrading an established user does
not force Welcome again. New schema errors remain visible and are not silently
rewritten. There is no automatic time/cadence reappearance policy. A separate
What's New surface is intentionally deferred.

## 13–20. Workflow and dependency discovery

**Implemented and build-checked:** Quick Render remains the short engine +
level + resources + optional music → validate → queue → MP4 route. It does not
require Chart Studio, Presentation Studio, profiles, manifests, or compiler
knowledge.

Chart Studio's entry points lead to its existing New LevelData/Open LevelData,
engine inspection, production Watch still preview, structural validation, and
gzip export controls. The Welcome copy does not promise engine-specific
authoring semantics where no matching schema exists.

Presentation Studio's entry point leads to the existing standalone
composition, which imports ordinary media, has its timeline and normal export,
supports vertical composition dimensions, and can record voiceover as regular
timeline audio. No engine, LevelData, Watch VM, or Sono-GCC is introduced by
that composition path. A dropped `.sonocomp` now opens directly from Welcome.

Learn includes vertical Shorts, local import, URL acquisition, microphone
recording, projects, export, and optional advanced renderer settings. URL
acquisition is described under its actual UI label, **Acquire media from URL**;
it is optional and creates local media. Voiceover guidance says capture begins
only after explicit user action. Missing yt-dlp, microphone devices, or
Sono-GCC do not disable local video editing or the VM render path. Advanced
VM/GCC/cache/performance details remain in help topics rather than the primary
home actions.

Recent-project behavior reuses `RecentProjects`; the store prunes inaccessible
render-project paths on load. It has no separate composition-recents catalog,
so no second recent-project database was added. A dropped `.sonoproject` now
routes to Project / inputs even when the current tab is Presentation Studio.

**Not supported:** this build has no OS file association and does not accept a
bare project path as a GUI command-line argument. Existing CLI project/render
commands remain available. File drop into the running application and in-app
project pickers are supported. Opening a composition from the picker or
Welcome drop does not initialize Sonolus.

## 21–27. Empty states, help, accessibility, and startup cost

**Implemented:** Welcome's recent-project area explains when there are no
recent render projects and points video users to the composition opener.
Presentation Studio itself already labels its standalone workflow and local
media import path; Chart Studio exposes New/Open LevelData controls. Quick
Render retains its existing preflight and beginner-facing error mapping.

**Inspected, not interactively verified:** the Welcome controls are ordinary
keyboard-focusable egui buttons inside a vertical scroll area, and the content
wraps in two columns. The environment does not provide desktop automation, so
keyboard-only operation, screen-reader semantics, high-DPI scaling, and small
window layouts were not manually exercised.

Displaying Welcome does not invoke a video decoder, microphone stream,
`WatchVm`, or Sono-GCC compilation in the Welcome UI path. Entering
Presentation Studio performs its existing deferred recovery/device discovery;
capture remains explicit. No dependency scan, telemetry, or network action was
added to startup. The application still initializes its existing renderer
stores and queue as part of normal desktop startup.

## 28–30. Validation and documentation

**Baseline:**

- `cargo test --locked --lib onboarding::tests` before edits: 2 passed.

**After edits:**

- `cargo fmt --all -- --check`: passed.
- `cargo check --locked --all-targets`: passed; existing warnings were a
  path-canonicalization warning and unavailable `rc.exe` for executable
  version metadata.
- `cargo test --locked`: passed: library 372 passed / 6 ignored, main 27,
  `arctan2` 3, `m0` 101, Watch scheduler 9, doc tests 0. No test harness abort
  occurred in this run; this does not establish that historical intermittent
  Windows aborts are fixed.
- Focused onboarding tests cover absent state, schema 1 completed/incomplete
  migration, complete/skip persistence, manual reset/replay eligibility, and
  rejection of a future schema.
- GUI tests cover skip persistence, opening a standalone `.sonocomp` from the
  first-run state with no sources, and dropping a render project while in the
  Presentation Studio tab.

`docs/GETTING_STARTED.md` was rewritten for actual current workflows.
`README.md` no longer claims Presentation Studio is still-frame-only.
`docs/ARCHITECTURE.md` now distinguishes `RenderProject` from the standalone
`CompositionProject` and lists onboarding persistence ownership.

## 31–32. Limits and deferred work

**Not tested in this environment:** a real desktop interaction, the Welcome
page at multiple window sizes/DPI scales, screen-reader behavior, a fresh
packaged installer launch, or actual OS file association launch. A release
artifact was not rebuilt or installed. Build/test success does not count as
visual or interactive validation.

**Intentionally deferred:** a separate recent-composition store, a new
What's New interruption, dedicated multi-step Chart Studio and Presentation
Studio tours, website documentation search, and OS file associations/bare-path
GUI launch. Existing Help topics and direct workspace entry cover current
workflow discovery without adding those systems.

### Evidence classification

| Claim | Classification |
|---|---|
| Welcome goals and routing | Implemented; unit-tested for file routing; build-checked |
| First-run, skip, completion, reset, and v1 migration | Implemented; unit-tested |
| Existing render recent list reuse | Manually inspected in code; not interactively exercised |
| Standalone composition independence | Existing architecture/model and render tests passed; new drop-routing test passed; no GUI interaction claimed |
| Help topics and documentation accuracy | Manually inspected; documentation updated |
| Layout, keyboard, DPI, and screen-reader UX | Not manually tested |
| Fresh packaged onboarding | Unavailable/not tested |
| OS association / GUI path argument | Not implemented; documented limitation |
