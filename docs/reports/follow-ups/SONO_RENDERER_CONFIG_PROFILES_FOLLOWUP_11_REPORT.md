# Sono-Renderer Follow-up #11: Full-Render Configuration Profiles

## Summary

Implemented named, persistent profiles as full snapshots of the existing
`RenderConfig`. Profiles add organization and reuse without replacing the
existing config-file, CLI, GUI, or renderer settings workflows. A profile is
resolved before rendering or queue submission; it is never consulted during
frame execution.

The shared implementation is in `src/config_profiles.rs`, and its user guide
is `docs/CONFIGURATION_PROFILES.md`.

## Architecture and schema

- `ConfigProfileManager` is the shared profile catalog used by the GUI and
  CLI. It supports create, list, inspect, select/deselect, update, duplicate,
  rename, reset, delete, import, export, and configuration resolution.
- Each profile has a stable generated ID, a unique case-insensitive display
  name, creation/update timestamps, and a complete user-facing `RenderConfig`
  snapshot. The profile does not introduce backend-specific behavior; VM and
  Sono-GCC remain options in the captured configuration.
- The local JSON catalog has `schema_version: 1` and stores the selected
  profile ID. Writes use a temporary file plus atomic replacement and a
  cross-process lock so GUI and CLI changes share one catalog.
- Portable JSON exports use the `sono-renderer-config-profile` format and
  their own schema version. Import creates a new local ID. Paths beneath the
  export file's directory are made relative; import resolves relative paths
  against that directory. External paths stay external and their assets are
  not packaged.
- This is the first profile schema, so there are no earlier profile files to
  migrate. Unsupported schema versions, missing required envelope fields,
  unknown profile/config fields, and invalid stored configs fail with a
  diagnostic rather than silently losing values. Legacy `RenderConfig` files
  retain their existing deserialization behavior.

## GUI and CLI

The desktop GUI has a full-render profile selector above its existing
settings tabs. The existing tabs remain the editor. Selecting a profile loads
its values into the normal controls; editing does not update the saved
profile automatically. The user can explicitly save changes, save as new,
duplicate, rename, discard edits, reset, delete, import, or export. Selecting
Current configuration clears the saved selection without changing the
working config. A selected profile is restored when the GUI starts.

CLI commands use the same store and manager:

```text
renderer profiles list
renderer profiles show "High Quality"
renderer profiles create "High Quality" --config renderer-settings.json
renderer profiles duplicate "High Quality" "High Quality Draft"
renderer profiles update "High Quality" --config updated-settings.json
renderer profiles rename "High Quality Draft" "Draft"
renderer profiles select "High Quality"
renderer profiles reset "Draft"
renderer profiles export "High Quality" high-quality.sono-profile.json
renderer profiles import high-quality.sono-profile.json --name "Portable HQ"
renderer profiles delete "Draft"
renderer profiles deselect

renderer validate-config --profile "High Quality"
renderer render-config --profile "High Quality"
renderer jobs submit --profile "High Quality" --enqueue-only
```

Use `--profile-store <path>` or `SONO_RENDERER_PROFILE_STORE` to select a
catalog path. Without `--profile`, existing config-file and no-profile
workflows remain available and do not implicitly switch to a saved profile.

## Configuration and jobs

Profiles store the existing render, execution, output, gameplay UI,
watermark, notification, and queue-default settings from `RenderConfig`.
Normal `RenderConfig` validation and render preflight remain authoritative.
Gameplay UI and watermark configurations are stored as their existing
profile/path inputs; their rendering behavior is not duplicated in the full-
render profile system. Sono-GCC cache placement remains an application cache
setting rather than a profile setting.

GUI queue submission records the profile ID and display name and passes the
resolved configuration to the job manager. CLI submission can explicitly
name a profile. Each queued job retains its effective config snapshot, so
changing or deleting its source profile later cannot change the queued job.
The renderer consumes the job snapshot for both VM and Sono-GCC execution.

## Secrets and portability

Notification credentials remain environment-variable references; the secret
value is not loaded into the profile or export. Export scrubs invalid
token-like values from the reference field, replacing them with the standard
environment-variable name. Other environment-variable values remain local to
the host. Exported project/media/UI/watermark paths are references only; the
assets themselves are not copied.

## Validation

- `cargo fmt --all --check` — passed.
- `cargo check --offline --all-targets` — passed.
- `cargo check --release --offline --all-targets` — passed.
- `git diff --check` — passed.
- `cargo test --offline --all-targets` — **300 passed, 0 failed, 3 ignored**.
  Ignored tests are existing offline diagnostics that require private fixture
  assets and write diagnostic artifacts.
- The suite includes profile CRUD, selection persistence, serialization,
  invalid names/configs, reset, import/export round-trip, relative paths,
  version and unknown/missing-field rejection, and secret scrubbing. It also
  includes CLI profile operations and profile job snapshot tests.
- The real-renderer pipeline test
  `video_export::tests::single_watch_export_audio_pipeline_offsets_and_failure_cleanup`
  passed. It rendered VM and Sono-GCC profile jobs, changed one profile and
  deleted another after enqueue, and verified the queued snapshots remained
  valid through actual rendering and output checks.
- A headless GUI behavior test applies a saved profile to the existing
  settings model and verifies that queue defaults are synchronized. GUI code
  compiled in debug and release checks. Interactive desktop click-through
  was not performed in this environment.

The compiler reports existing deprecation/dead-code warnings in runtime,
profiling, export-progress, and cache code. No new profile-related compiler
warnings were reported.

## Remaining limitations

- There is no migration from earlier profile schemas because this introduces
  the initial profile schema. Future schema changes need explicit migrations;
  unsupported versions currently fail clearly.
- The GUI was tested through its headless configuration behavior and
  compilation, not by visually operating the desktop window.
- Portable exports do not bundle referenced media/assets, and paths outside
  the export directory remain absolute.
