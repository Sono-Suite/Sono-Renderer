# Project workflow

Sono-Renderer projects are lightweight JSON documents with the `.sonoproject`
extension. They describe the render subject and may select a saved full-render
profile. They do not contain engine archives, resource packages, audio, video,
or rendered output.

```text
renderer project inspect chart.sonoproject
renderer project validate chart.sonoproject
renderer project validate chart.sonoproject --json
renderer render-project chart.sonoproject --output renders/chart.mp4
renderer render-project chart.sonoproject --profile "High Quality" --enqueue-only
```

`render-project` resolves the project and profile, validates the effective
inputs, then places a concrete `RenderConfig` snapshot in the persistent job
queue. Later project or profile edits do not change that queued job. The
existing `render-config`, `validate-config`, `jobs submit`, and direct GUI input
workflow remain available.

## Schema and separation

Version 1 has the format identifier `sono-renderer-project`, metadata, input
references, an optional profile reference, and narrow project overrides. The
input references cover engine, level, primary resources, music, cover, MV, and
the renderer's separate particle/effects/background resource packages.
Unknown JSON keys at the project, metadata, inputs, profile reference, and
overrides levels are preserved when the project is saved. Schema version 0 is migrated using current
defaults; unsupported future versions are rejected without rewriting the file.
Project files are limited to 16 MiB.

The profile ID is authoritative, so renaming a profile does not break a
project. The display name is retained for humans and old name-only references.
At job creation, the current revision of that profile is used. A deleted or
unavailable required profile is an error; the renderer never chooses a
similarly named profile. The user can repair the selection in the GUI or pass
an explicit `--profile` to `render-project`. The resulting job snapshot is
independent of later profile changes.

Projects do not store global notification preferences, Discord credentials,
or other application secrets. Profile configuration continues to follow the
shared `RenderConfig` model. The project UI can explicitly override render
range, resolution/frame rate, and enabled render layers; its output path is a
project default.

## Path rules

Relative asset references are resolved against the directory containing the
project. Saves convert absolute paths on the same filesystem root to relative
references and serialize separators as `/`, so projects can move between
directories and operating systems when their relative assets move with them.
Absolute references remain machine-local. A Windows absolute path opened on a
Unix-like system, or a Unix-rooted path opened on Windows, is reported as
unresolvable and must be repaired. Relative inputs may intentionally refer to
shared files outside the project directory; they are only read as render
inputs. A relative output path that escapes the project directory is rejected.

Missing or malformed assets are reported by project validation. Opening a
project does not execute code. Importing a file only parses bounded JSON and
references; actual rendering requires an explicit render/job action. Saves use
a same-directory temporary file, flush the file, and atomically replace the
target. The recent-project list is separately versioned, capped at 12 entries,
and stale paths are discarded without blocking startup.

Preflight also resolves the selected profile and runs the shared compatibility
analyzer. When the effective configuration enables BGM, it requires an
accessible music file. It checks the output parent and warns if the target
already exists, since queued jobs will not overwrite existing outputs.

## GUI discovery

The main desktop GUI provides New Project, Open Project, Save, Save As, and
recent projects. Dropping one valid project opens it. Dropping input files or a
directory uses the existing engine, level, and SCP parsers, audio/video
signatures, and image decoder to identify candidates. A directory scan stops
after 256 files and two directory levels. A required role is auto-filled only
when one distinct candidate exists for engine, level, and resources. Multiple
candidates remain explicit choices; unsupported or inaccessible files are
listed with a reason. Optional music, cover, and MV choices are not inferred
from filenames.

The project panel validates inputs and compatibility through the shared
compatibility analyzer, allows profile selection/repair, and creates a queued
render job from a resolved snapshot. The existing direct input fields and
Render MP4 action continue to work without creating a project.

Cover is retained as a project presentation reference. The current render
pipeline does not consume a cover asset, so it is not injected into gameplay
frames or the output video.

URL-acquired files follow the same `music` and `mv` path semantics as files
selected from disk. The desktop stores them under `media/` beside a saved
project (or under per-user renderer data without a saved project); CLI project
downloads default to the project's sibling `media/` directory. The project
stores only the resulting path reference. It does not store the source URL or
embed the media bytes. See [URL media acquisition](MEDIA_ACQUISITION.md).
