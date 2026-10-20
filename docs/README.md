# Sono-Renderer Documentation

This is the canonical index for project documentation. The root
[README](../README.md) is the project landing page; this page organizes user
guides, technical references, development material, and historical reports.
Reports preserve the evidence and project state from their original review
periods. Use current guides and [known limitations](KNOWN_LIMITATIONS.md) for
present-day behavior.

## Start here

- [Getting Started](GETTING_STARTED.md) — open Quick Render and make a first
  export.
- [Installation and platform status](INSTALLATION.md) — releases, portable
  installs, FFmpeg, storage, and supported targets.
- [Workspace navigation and commands](WORKSPACE_COMMANDS.md) — default
  navigation, shortcuts, CLI entry points, and feature registration.
- [Troubleshooting](TROUBLESHOOTING.md) — common input, render, and export
  problems.

## Rendering workflows

- [Project workflow](PROJECT_WORKFLOW.md) and
  [configuration profiles](CONFIGURATION_PROFILES.md).
- [Shared settings and configuration](USER_CONFIGURATION.md).
- [Render jobs](RENDER_JOB_MANAGER.md), [render history](RENDER_HISTORY.md),
  and [render manifests](RENDER_MANIFESTS.md).
- [Program Preview](PROGRAM_PREVIEW.md), [render diagnostics](RENDER_DIAGNOSTICS.md),
  and the [compatibility analyzer](COMPATIBILITY_ANALYZER.md).
- [URL media acquisition](MEDIA_ACQUISITION.md),
  [notifications](NOTIFICATIONS.md), and [cache management](CACHE_MANAGEMENT.md).
- [Optional score-count animation](OPTIONAL_SCORE_COUNT_ANIMATION.md) documents
  the gameplay UI profile hook and its validation.
- [Gameplay UI profiles](../ui-profiles/README.md),
  [profile asset inventory](../ui-profiles/ASSET_USAGE.md), and
  [watermark profiles](../watermark-profiles/README.md). These package guides
  remain beside the profile data they describe.

## Architecture and reference

- [Architecture](ARCHITECTURE.md),
  [architectural invariants](ARCHITECTURAL_INVARIANTS.md), and
  [general platform architecture](GENERAL_PLATFORM_ARCHITECTURE.md).
- [M0 format and design notes](M0-design.md) and
  [reference source provenance](reference/REFERENCE_SOURCES.md).
- [Sono-GCC implementation and coverage](development/sono-gcc/SONO_GCC_IMPLEMENTATION_REPORT.md),
  [operation coverage](development/sono-gcc/SONO_GCC_COVERAGE.md), and
  [experiment guide](development/sono-gcc/EXPERIMENT_README.md).
- [Current limitations](KNOWN_LIMITATIONS.md) describe validation gaps and
  compatibility boundaries.

## Compatibility and technical investigations

- [Sonolus 1.1.4 capability audit](SONOLUS_V1_1_4_CAPABILITY_AUDIT.md) and
  [oracle arity audit](SONOLUS_V1_1_4_ORACLE_ARITY_AUDIT.md).
- [Particle compatibility handoff](PARTICLE_COMPATIBILITY_HANDOFF.md),
  [particle forensic audit](PARTICLE_FORENSIC_AUDIT.md), and
  [SDB particle differential](SDB_PARTICLE_DIFFERENTIAL.md).
- [Skin render-mode compatibility](SKIN_RENDER_MODE_COMPATIBILITY.md),
  [SFX decoding](SFX_FFMPEG_DECODING.md),
  [hold audio and background rotation](ENGINE_INDEPENDENT_HOLD_AUDIO_AND_BACKGROUND_ROTATION.md),
  and [single-Watch export](SINGLE_WATCH_EXPORT.md).
- [MV export](MV_EXPORT.md), [alpha compositing audit](MV_ALPHA_COMPOSITING_AUDIT.md),
  and [presentation history audit](MV_PRESENTATION_HISTORY_AUDIT.md).
- [Dynamic-stage draw audit](DYNAMIC_STAGE_DRAW_AUDIT.md),
  [combo glow calibration](COMBO_GLOW_LIGHT_CALIBRATION.md),
  [hold-SFX forensic audit](HOLD_SFX_FORENSIC_AUDIT.md), and
  [GUI export panic diagnostics](GUI_EXPORT_PANIC_DIAGNOSTICS.md).
- [Pre-UI storage audit](PRE_UI_STORAGE_AUDIT.md) and
  [archival dependency audit](ARCHIVAL_DEPENDENCY_AUDIT.md) preserve storage
  and local-reference evidence.
- [Sono-Overlay investigations and parity reports](reports/investigations/sono-overlay/SONO_OVERLAY_EXACT_PARITY.md).

## Development, testing, and operations

- [Performance Lab](PERFORMANCE_LAB.md), [Regression Lab](REGRESSION_LAB.md),
  and [export lifecycle](EXPORT_LIFECYCLE_MILESTONE.md).
- [Native GUI milestone](NATIVE_GUI_MILESTONE.md) and
  [admin console](ADMIN_CONSOLE.md).
- [Release process](RELEASE_PROCESS.md). The release-note template stays at
  [`release-notes/TEMPLATE.md`](../release-notes/TEMPLATE.md), the path consumed
  by the release workflow.
- [Localization guide](../locales/README.md) documents locale metadata and
  message conventions. The release packager copies it as `LOCALIZATION.md`.
- [Chart Studio](CHART_STUDIO.md) and
  [Presentation Studio](PRESENTATION_STUDIO.md) describe retained
  implementations whose routes are hidden from the default interface.
  Related references cover [tempo analysis](CHART_STUDIO_TEMPO_ANALYSIS.md),
  [Presentation Studio assets](PRESENTATION_STUDIO_ASSETS.md),
  [audio input](PRESENTATION_STUDIO_AUDIO_INPUT.md), and
  [subtitles](PRESENTATION_STUDIO_SUBTITLES.md).

## Follow-ups and historical reports

The [reports index](reports/README.md) catalogs preserved follow-ups, audits,
closures, integration reports, and benchmarks. The original report contents
remain intact; no completion or supersession status is inferred from their
location.

The [archived README](archive/ai_readme.md) is retained for historical context.
Use this index and the root README for current project guidance.

## Documentation retained beside fixtures and packages

- `artifacts/nle-validation/README.md` records generated media and hashes next
  to those artifacts.
- `TestingSuite/` contains local test inputs and reverse-engineering material;
  its fixture documentation remains with those inputs.
- Third-party repository READMEs and license notices remain with their
  respective vendored or reference materials.
