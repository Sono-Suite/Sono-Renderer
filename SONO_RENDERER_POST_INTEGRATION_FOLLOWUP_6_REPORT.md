# Sono-Renderer Post-integration Follow-up #6

## Delivered

Added a versioned `.sonoproject` JSON workflow on top of the existing
`RenderConfig`, profile catalog, and persistent job manager. Projects describe
render inputs and optional project overrides. Profiles remain reusable
renderer preferences. At job creation, project references and the selected
profile/current settings resolve to a cloned `RenderConfig`; the queue stores
that immutable snapshot.

The version 1 schema stores engine, level, primary resources, music, cover,
MV, and separate particle/effects/background resource references. It stores a
stable profile ID plus a display name, project metadata, output default, and
optional range/resolution/FPS/layer overrides. Global notifications and
Discord credentials are not copied into project files. Cover references are
retained for project organization; the current video/gameplay pipeline does
not consume a cover.

Schema version 0 migrates through defaults. Unknown JSON fields are retained
at the project, metadata, inputs, profile reference, and overrides levels. Unknown future schema
versions are rejected without rewriting the source. Project files are size
limited to 16 MiB. Saving uses a same-directory temporary file, flushes it,
and atomically replaces the target; Unix saves also sync the parent directory.

Relative asset paths resolve from the project directory and are saved with
portable `/` separators. Absolute paths stay machine-local. Cross-OS absolute
references are reported as needing repair. Relative asset references may
intentionally point outside the project folder; relative output paths may not
escape it. Opening a project performs no code execution. A missing required
profile is an error; profile renames resolve by stable ID, and a profile's
current revision is used when creating a job. Once queued, profile or project
changes do not mutate the job snapshot.

## GUI and discovery

The existing main GUI now includes New Project, Open Project, Save, Save As,
recent projects, project validation, profile selection/repair, project
overrides, and “Create render job.” The direct input fields and direct Render
MP4 action remain available.

Drag/drop uses the existing engine, level, and SCP loaders, file signatures
for audio/video, and the image decoder for cover candidates. Engine and level
parsers take precedence over the generic ZIP/SCP reader so an engine archive
is not also mistaken for a resource package. Required roles are filled
automatically only when engine, level, and resources each have one distinct
candidate. Ambiguous roles remain explicit selectors. Multiple SCP candidates
can be assigned to the primary, particle, effects, or background slots by the
user. Directory scans are limited to 256 files and two directory levels.
Recent-project history is versioned, capped at 12 entries, and stale entries
are discarded without blocking startup.

The preflight calls the shared compatibility analyzer. It reports missing or
malformed inputs and checks project profile/config resolution without
rendering. If resolved settings enable BGM, it requires an accessible music
file; output parent and existing-target conditions are also checked. The
`project inspect`, `project validate [--json]`, and
`render-project` CLI commands were added; existing direct config and
`validate-project` commands remain intact.

## Validation performed

- `cargo check --all-targets` passed. The build still prints existing warnings
  in unrelated runtime/profiling code.
- `cargo test project::tests --lib` passed: 14 passed, 0 failed, 1 ignored.
  Coverage includes schema migration/version rejection, unknown-field and
  portable-path round trips, atomic replacement, moved project paths, invalid
  values/output traversal, parser-based ambiguous mappings, stable profile
  identity and missing-profile handling, and queued snapshot immutability.
- The Next Sekai fixture preflight test loaded the real engine, SCP, compressed
  level, and MP3, discovered their roles, and ran compatibility analysis.
  Result: 640 level entities, 16 resources, and a compatibility report.
- The opt-in real-render integration test was run explicitly and passed. It
  saved/reopened a project, resolved config, queued a 128x72 CPU render at 2
  FPS for 0.5 seconds with fixture BGM, and verified a non-empty MP4 output.
- The CLI `project validate` command passed against the same fixture and
  reported 9 compatibility assessments with 0 structural findings.
- The CLI `render-project` command completed a fixture job through the
  persistent queue and wrote an MP4.
- GUI code was build/test compiled, but the native GUI was not interactively
  exercised in this environment.

## Limitations

Input discovery only identifies accepted asset roles; it does not infer which
of several SCP archives is semantically the skin/effects/particles/background
pack. The user assigns ambiguous SCPs explicitly. Audio/video recognition is
signature-based for the formats exposed by the current input workflow and is
not a full decode probe. Cover support is reference-only because the render
pipeline has no cover input. Projects intentionally remain references rather
than bundling large assets.

See [project workflow documentation](docs/PROJECT_WORKFLOW.md) for the schema,
path rules, and CLI examples.
