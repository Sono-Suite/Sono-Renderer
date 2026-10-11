# Full-render configuration profiles

Full-render profiles are named snapshots of the shared `RenderConfig`. They
are distinct from gameplay HUD profiles and watermark profiles: those remain
asset/profile inputs contained within a full-render profile.

## GUI behavior

The profile selector sits above the existing renderer tabs. **Current
configuration** means the working settings are not selected from a saved
profile. Selecting a profile copies its configuration into the existing tabs.
Those tabs remain the editor; the app does not maintain a second settings
form. Changes are marked unsaved and do not alter the saved profile until
**Save changes** is pressed. **Discard edits** restores the saved snapshot.
**Save as new**, **Duplicate**, **Rename**, **Reset to renderer defaults**,
**Delete**, **Import**, and **Export** operate on saved profiles explicitly.
Deleting a profile never deletes or changes jobs already created from it.

The selected profile ID is shared in the profile catalog. The GUI restores it
on startup. Selecting **Current configuration** clears that selection while
leaving the current working settings intact.

## CLI

The CLI and GUI use the same catalog and case-insensitive profile names or
stable profile IDs:

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

`profiles create` without `--config` copies the active profile, or starts from
renderer defaults if none is selected. Existing config-file commands continue
to use their file unchanged unless `--profile` is supplied. Profile resolution
happens once before validation/rendering. A profile selected with `profiles
select` is the GUI's startup selection; CLI render and queue operations use
explicit `--profile` selectors so an unrelated saved selection never silently
overrides an explicitly supplied config file.

Use `--profile-store <path>` to point GUI-launched CLI commands at a specific
catalog, or set `SONO_RENDERER_PROFILE_STORE`. The default is
`%LOCALAPPDATA%/Sono-Renderer/profiles.json` on Windows and
`$XDG_CONFIG_HOME/sono-renderer/profiles.json` on other platforms, falling back
to `$HOME/.config/sono-renderer/profiles.json`.

## Persistence and format

The local catalog is an atomically replaced JSON document with schema version
1, stable IDs, unique display names, timestamps, active selection, and a full
config snapshot. Writes use a cross-process lock so GUI and CLI mutations do
not silently overwrite each other. Profile names are trimmed, limited to 64
characters, and reject control characters and path separators. Names are
unique without regard to case.

Portable profile exports use a separate envelope:

```json
{
  "format": "sono-renderer-config-profile",
  "schema_version": 1,
  "profile": {
    "name": "High Quality",
    "config": { "...": "complete RenderConfig" }
  }
}
```

The exported JSON has no local profile ID; importing creates a new stable ID
and rejects conflicting names. Paths within the export directory are stored
relative to the exported file. External paths remain absolute because the
renderer does not package project, media, UI, or watermark assets. Import
resolves relative config/UI/watermark paths beside the export file. Users can
edit paths in the normal renderer tabs and explicitly update the profile.

The export format includes its schema version. Unsupported future versions,
unknown envelope fields, unknown top-level `RenderConfig` fields, malformed
profiles, and missing required envelope fields return diagnostics. The
application does not silently import a newer schema while dropping settings.
New schemas can add explicit migrations without changing legacy
`RenderConfig` file behavior.

## Queue snapshots and secrets

Submitting a profile to the render queue resolves the profile immediately.
Each job stores the effective `RenderConfig`, selected profile ID/name, and
all existing job execution metadata. Later profile edits or deletion do not
change the queued job. The profile is attribution only after submission; the
job renderer consumes its captured config.

Notification settings travel with the profile. Discord credentials continue
to be read from the configured environment variable. The secret itself is
never read into the profile. If the credential field is accidentally set to a
value that is not a valid environment-variable name, the manager replaces it
with the standard environment-variable name before local persistence or
export. Other environment variables remain on the host that launches the
renderer.

Profiles cover settings that exist in `RenderConfig`, including VM/Sono-GCC,
backend, timing/output, UI, watermark, notifications, and queue defaults. The
Sono-GCC cache location remains an automatic application cache setting rather
than profile content. VM remains available in every profile.

## Validation

Profiles are structurally validated on create, update, import, and resolution.
File-backed inputs and output requirements are validated by the same
`RenderConfig` preflight used for ordinary renders. Full profile values are
copied once to a render or job; profiles are not deserialized during frame
execution. Unit tests cover CRUD, name validation, selection persistence,
round-trip import/export, schema and unknown-field handling, and credential
scrubbing. CLI tests exercise profile management and profile-based queue
submission. The real-renderer fixture renders VM and Sono-GCC jobs from two
profiles, then changes one profile and deletes the other before execution to
verify job snapshot stability.

The main GUI profile controls are build-checked and use the same tested
`ConfigProfileManager`. Interactive click-through has not been performed in a
desktop session.
