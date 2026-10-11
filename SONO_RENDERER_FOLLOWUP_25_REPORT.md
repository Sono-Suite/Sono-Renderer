# Sono-Renderer Follow-up #25 — First-Run Experience Report

## Scope and repository findings

The current application is a desktop Sonolus Watch renderer with a large
multi-tab workspace, direct MP4 export, project/profile workflows, a persistent
render queue, gameplay UI profiles, and a real-renderer still-frame Program
Preview. It does **not** currently contain the media bin, multi-track timeline,
continuous preview monitor, inspector, or nonlinear video editing workflow
described in the prompt. The onboarding now says this plainly instead of
presenting those controls as if they existed.

## Implemented

- **First launch and welcome:** A small welcome screen introduces rendering,
  HUD customization, and still-frame inspection. It offers Quick Render, Open
  Project, gameplay UI settings with an optional tour, Learn/help, and access
  to the full workspace.
- **Quick Render:** Uses the existing `RenderConfig`, input widgets, persistent
  job manager, renderer, and history path. It does not create a beginner-only
  config, renderer, or queue. The page explains required inputs, optional
  music behavior, and optional media/resource overrides.
- **Input discovery:** Reuses existing file/folder drag-and-drop discovery.
  Unique role mappings are applied; ambiguous matches remain for the user to
  choose. The onboarding copy describes this behavior.
- **Preflight:** Runs the existing export-input validation and compatibility
  analyzer, checks the output path and collision, and inspects installed
  FFmpeg/FFprobe without installing anything. Export retains existing lazy
  FFmpeg provisioning behavior. Findings and technical details are available
  separately.
- **Render summary and progress:** Shows engine, level, resources, enabled
  music, output, resolution, FPS, and duration. In-progress text comes from the
  actual job phase/frame progress.
- **Success and failure:** Shows the output location and Open Video/Open Folder
  actions. It remembers the first successful job/output locally. Failures have
  a plain-language lead and expandable technical diagnostics; an output
  collision asks the user to choose another path.
- **Progressive disclosure:** Quick Render hides advanced tabs while the full
  workspace remains directly reachable. “Find a setting” navigates to existing
  sections for terms such as GCC, cache, watermark, Discord, LIFE, and FPS; it
  does not duplicate settings.
- **Tooltips and terminology:** Input labels explain engine, level, resource,
  music, MV, and output roles. Sono-GCC help reflects the current optional
  eligible-region compilation and VM fallback behavior. Help differentiates
  project, profile, job, and history.
- **Getting Started/help:** An offline searchable help window covers first
  render, inputs, projects/profiles, jobs/history, Sono-GCC, current gameplay
  presentation, and CLI usage. The optional five-step tour navigates actual
  input, gameplay UI, Program Preview, render settings, and queue panels. It
  supports Back, Next, Skip, Finish, and window close.
- **Persistence:** `src/onboarding.rs` owns a local JSON preference file with a
  format identifier and schema version. It is saved using a same-directory
  temporary file and atomic persistence. Unsupported future schemas are
  reported and the file is left untouched until the user explicitly chooses
  Reset onboarding. Preferences contain onboarding state and first-success
  output metadata only, not RenderConfig or credentials.
- **Documentation:** The README has a beginner-first Quick Start and links to
  `docs/GETTING_STARTED.md`, which documents current input requirements,
  discovery, preflight, FFmpeg provisioning, projects/profiles, CLI, and the
  actual presentation-tool limitations.

## Acceptance matrix

Statuses below describe this pass. “Unavailable” means the requested lesson or
interaction depends on a feature that does not exist in the current checkout.

| # | Area | Result |
|---:|---|---|
| 1 | Original first-run experience | **Manually inspected in code:** the app opened into the full advanced workspace. |
| 2 | Usability problems | **Manually inspected:** the first screen exposed many specialized tabs with no beginner entry point. |
| 3 | New first-run experience | **Implemented; unit-tested for initial state.** |
| 4 | Welcome/start screen | **Implemented; build-checked.** |
| 5 | Quick Render | **Implemented over production config and queue; not end-to-end queue-tested.** |
| 6 | Input discovery | **Existing implementation reused; not interactively tested in this pass.** |
| 7 | Drag/drop | **Existing implementation reused; ambiguous roles are surfaced; not interactively tested.** |
| 8 | Input validation | **Implemented through shared validation and compatibility analysis; unit guidance tests pass.** |
| 9 | Render summary | **Implemented; build-checked.** |
| 10 | Progress UX | **Implemented from existing job phases and frame progress; not manually exercised.** |
| 11 | Success UX | **Implemented, including output actions and persisted first-success reference; not render-tested.** |
| 12 | Failure UX | **Implemented with actionable leading guidance and expandable diagnostics; unit-tested for common messages.** |
| 13 | Progressive disclosure | **Implemented as a Quick Render entry plus the unchanged advanced workspace.** |
| 14 | Settings organization | **Search navigation implemented; existing advanced tab organization remains.** |
| 15 | Tooltips | **Input and key execution controls have explanations; not every advanced control was audited.** |
| 16 | Contextual help | **Implemented as searchable offline help.** Content remains maintained in GUI code rather than generated from a shared source. |
| 17 | Terminology | **Updated in new user-facing copy; no repository-wide terminology audit performed.** |
| 18 | Projects/profiles | **Explained in help/docs; existing workflow reused.** |
| 19 | Sono-GCC | **Explained accurately in help and tooltip; no speedup claim.** |
| 20 | Presentation introduction | **Implemented truthfully for gameplay HUD profiles and still-frame preview.** |
| 21 | Presentation Studio tour | **Requested NLE tour unavailable.** A distinct optional tour targets the actual gameplay UI/preview/settings/queue controls. |
| 22 | First-project/tutorial workflow | **Unavailable:** no multi-track composition/editor exists. |
| 23 | Keyframe/easing tutorial | **Unavailable:** no user-facing timeline editor exists. Existing profile animation is not represented as a fake editor. |
| 24 | LIFE tutorial | **Unavailable:** no corresponding Studio workflow exists. |
| 25 | Score/combo/judgement lessons | **Unavailable as guided editor lessons:** these depend on the absent Studio UI. |
| 26 | Asset-pack introduction/browser | **Limited:** help mentions profile assets, but no pack browser/preview/selection UI was added. |
| 27 | Templates/examples | **Not added:** no suitable complete Studio templates exist in the current UI. |
| 28 | Learn/help system | **Implemented:** searchable offline topics for implemented features. |
| 29 | CLI onboarding | **README and help topic point to current `render-video` usage; CLI behavior unchanged.** |
| 30 | Error-message improvements | **Quick Render common input/FFmpeg errors get plain-language leads; broad CLI errors not changed.** |
| 31 | Empty states | **Quick Render has explicit input and status guidance; no global panel-by-panel empty-state audit.** |
| 32 | Accessibility/usability | **Scrollable Quick Render and plain text labels added; no screen-reader, keyboard-only, or DPI manual audit.** |
| 33 | Onboarding persistence/reset | **Implemented and unit-tested, including unsupported schema handling and reset/replay controls.** |
| 34 | Documentation | **README and Getting Started guide updated.** |
| 35 | Beginner-path integration tests | **Not run:** no full valid engine fixture was submitted through the GUI queue in this pass. |
| 36 | Presentation onboarding tests | **Unit-tested for GUI defaults/help-related helpers; interactive tour controls not exercised.** |
| 37 | Real-engine first render | **The serial library suite passed real-engine preview/pipeline tests; a first Quick Render MP4 job was not submitted.** |
| 38 | GUI manual testing | **Unavailable/not performed:** GUI was build-checked but not launched and exercised interactively. |
| 39 | Backward compatibility | **The existing advanced workspace, RenderConfig, queue, and CLI paths were left in place; build and targeted tests pass. No GUI-mediated end-to-end regression render was run.** |
| 40 | Known limitations | **No nonlinear editor, first-project timeline tutorial, template browser, or comprehensive control/help audit.** |

## Validation performed

- `cargo fmt --all` — **passed**.
- `cargo check --offline` — **passed**. Existing warnings remain for a deprecated
  atomic method in `src/runtime.rs` and unused `ProgressTracker::pulse` in
  `src/export_progress.rs`.
- `cargo test --offline --lib gui::tests -- --test-threads=1` — **18 passed**.
- `cargo test --offline --lib onboarding::tests -- --test-threads=1` — **2
  passed**.
- `cargo test --offline --lib -- --test-threads=1` — **273 passed, 4 ignored**
  out of 277. Ignored tests require private fixtures or explicitly run a real
  queue/FFmpeg export. This includes real Next Rush preview and renderer
  pipeline tests.
- `cargo test --offline --test m0 --test arctan2 --test watch_scheduler --
  --test-threads=1` — **113 passed** across the three integration targets.
- `cargo run --offline -- --help` and `cargo run --offline -- render-video
  --help` — **passed**; current direct-render syntax and options were inspected.
- `cargo clippy --offline --all-targets` — **completed successfully with existing
  repository warnings** (143 lib/test lint warnings in aggregate, plus main and
  integration warnings). No warning pointed to `src/onboarding.rs`.
- `cargo build --release --offline` — **passed**.
- `git diff --check` — **passed** (Git printed existing line-ending notices).
- A default parallel `cargo test --offline` run terminated with Windows
  `STATUS_HEAP_CORRUPTION` during the parallel library harness. The complete
  library suite then passed serially; the crash did not reproduce. The binary
  unit-test target was not separately run.
- No Quick Render GUI job, CLI render subprocess, or interactive GUI usability
  session was performed.

## Files for review

- `src/onboarding.rs` — versioned local onboarding preferences.
- `src/ffmpeg.rs` — non-installing FFmpeg inspection for preflight.
- `src/gui.rs` — welcome, Quick Render, help/tour, settings search, and guidance.
- `docs/GETTING_STARTED.md` and `README.md` — beginner workflow.

The onboarding is additive. The current workspace and existing CLI commands
remain available. A real Presentation Studio tutorial should be added alongside
the actual editor when that product surface exists; teaching nonexistent
timeline controls here would mislead first-time users.
