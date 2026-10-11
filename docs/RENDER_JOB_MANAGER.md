# Render job manager

The persistent job manager is shared by the desktop GUI and CLI. A job owns a
configuration snapshot and a stable ID; render execution still goes through
`RenderConfig::render_video_controlled`, so Watch, VM, Sono-GCC, frame output,
FFmpeg, gameplay UI, watermarks, and existing caches stay on the renderer's
normal path.

## CLI

Submit a shared renderer configuration and wait for the queue to drain:

```text
renderer jobs submit renderer-settings.json
```

Queue without starting workers, then inspect or run it later:

```text
renderer jobs submit renderer-settings.json --priority high --enqueue-only
renderer jobs list
renderer jobs inspect <job-id>
renderer jobs run
renderer jobs cancel <job-id>
renderer jobs retry <job-id>
renderer jobs remove <job-id>
renderer jobs reorder <job-id> <zero-based-position>
renderer jobs pause
renderer jobs resume
renderer jobs cancel-queued
renderer jobs clear-terminal
```

`submit` and `retry` run the queue by default; add `--enqueue-only` to leave
the job queued. `jobs run --config renderer-settings.json` supplies scheduler
settings explicitly. Without it, `run` uses the first queued job's saved
`job_manager` settings. Failed jobs do not stop other queued work; the command
reports failures after the queue becomes idle.

All commands accept `--store <path>` after `jobs` to select a store explicitly.
The environment variable `SONO_RENDERER_JOB_STORE` provides the same override.
Otherwise Windows uses `%LOCALAPPDATA%\Sono-Renderer\jobs.json`; other systems
use `$XDG_STATE_HOME/sono-renderer/jobs.json`, then
`$HOME/.local/state/sono-renderer/jobs.json`.

## GUI

The **Job manager** tab adds the current render configuration, controls the
queue, and lists state, priority, attempt, backend, progress, diagnostics,
output hashes, logs, and the immutable per-job configuration snapshot. It can
cancel, retry failed jobs, remove terminal jobs, reorder queued jobs, pause or
resume scheduling, cancel all queued jobs, and clear terminal records. Existing
one-off **Render MP4** remains available when the queue has no active render.

Changing the render form after submission only affects later jobs. The queue
tab's **Maximum concurrent renders** and **Default priority** are saved in the
shared `RenderConfig.job_manager` object. The default concurrency is one because
each render can hold GPU resources, decoded media, frame buffers, and an FFmpeg
process. The supported limit is 1..=8. Priority ordering uses aging so an old
low-priority job eventually passes newer high-priority submissions.

## Lifecycle and recovery

Jobs transition through `queued`, `preparing`, `compiling`, `rendering`,
`encoding`, then a terminal state (`completed`, `failed`, or `cancelled`). The
compiling state is reported from Sono-GCC's cache-miss callback; cache hits do
not claim a compile occurred. Progress and ETA come from the existing typed
export-progress events.

Cancellation stops new work immediately and uses the renderer's cooperative
control in Watch, frame rendering, and owned media children. Queue pause stops
new jobs; an active render is allowed to continue. The persistent JSON store is
atomically replaced under a cross-process lock. GUI and CLI instances refresh
and merge store updates, and per-job claim files prevent both from executing
the same queued job. A cancellation requested from another process is picked
up by the active worker.

On startup, an active job whose recorded worker process is gone becomes
`failed` with an `interrupted` diagnostic. The manager never converts an
unconfirmed render into `completed`. Queued work remains queued and is resumed
when the GUI starts or `renderer jobs run` is invoked. Failed jobs require a
manual retry; prior diagnostics and attempt history remain attached.

Each attempt renders to an adjacent job-specific staging MP4. The final path is
published with a no-clobber hard link, so existing files are never silently
replaced. A failed or cancelled render removes its partial staging output. If
the final publish step itself fails, the complete staged result is retained and
its path is in the job diagnostic. Retry refuses to run while a final output
file already exists.

Job records include the renderer version, input paths, complete render config
snapshot, selected CPU/WGPU and VM/Sono-GCC modes, progress, bounded logs,
failure classification, and successful frame/timeline/RGB/SFX hashes. The
configuration does not contain a Discord bot token; credentials continue to
come from the configured environment variable.

## Notifications

The queue uses the existing notification dispatcher and its per-backend event
filters. It adds `job_queued`, `job_started`, `job_compiling`, `job_rendering`,
`job_completed`, `job_failed`, and `job_cancelled`; it also emits the existing
`render_started`, `render_progress`, `render_completed`, and `render_failed`
events for compatibility. Compilation notifications occur only on an actual
Sono-GCC cache miss. No Discord behavior is implemented in the manager.

## Validation

The job manager unit tests cover snapshots, unique identity, state transitions,
priority aging, cross-manager storage merging, cancellation, retry history,
crash recovery, and output reservations. The CLI dispatch test exercises
submit/list/inspect/cancel against a temporary persistent store. The renderer
integration fixture submits multiple jobs through the manager, checks that a
failed job does not stop successful siblings, validates staging publication,
and compares VM and Sono-GCC render hashes. The test uses the repository's
managed FFmpeg fixture and skips only when that installation is absent. It
also asserts overlapping execution intervals with two workers, restores a
missing engine and retries that saved job successfully, and cancels a live
render while checking that neither final nor partial output remains.

Validation on 2026-10-06: the final integration pass reran
`cargo test --offline --all-targets` and reported 301 passed, 0 failed, and 3
ignored across the library, CLI, real-engine, renderer, and scheduler targets.
The ignored tests are private-corpus diagnostics that write audit artifacts.
The real-renderer queue test exercised the queue-to-VM/Sono-GCC-to-FFmpeg path,
profile snapshots, retry, concurrent jobs, and live cancellation.

Interactive GUI operation and native/Discord delivery still require a running
desktop session and are not exercised by headless tests.
