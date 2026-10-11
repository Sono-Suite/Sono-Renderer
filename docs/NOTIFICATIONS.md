# Renderer notifications

Sono-Renderer emits export and render-job lifecycle events to an optional notification
dispatcher. Event submission uses a fixed-capacity queue of 64 items and never
waits for a desktop service or network request. If the queue is full, the new
notification event is dropped. Backend errors and panics are isolated on the
worker thread; the render result is unchanged.

The renderer reports `render_started`, `render_progress`, `render_completed`,
and `render_failed`. The persistent job manager adds `job_queued`,
`job_started`, `job_compiling`, `job_rendering`, `job_completed`, `job_failed`,
and `job_cancelled`. Progress is disabled by default. `job_compiling` is emitted
only for an actual Sono-GCC cache miss; a cache hit does not claim compilation.

## Configuration

Notifications are part of the shared `RenderConfig` used by the desktop
**Notifications** tab and the `default-config`, `validate-config`, and
`render-config` CLI commands. The GUI's existing **Save configuration...** and
**Load configuration...** controls persist and restore the same settings.
Older configuration files without a `notifications` object receive defaults.
If only the notification object has an invalid shape or enum value, that
section is disabled and reported as a non-fatal diagnostic; the other render
settings remain usable.

For the positional `render-video` command, supply an independent
`NotificationConfig` JSON file with `--notifications-config`. Add
`--no-notifications` to disable it for one invocation. The config-file
`render-config` command uses the `notifications` field inside its full
`RenderConfig` file. A standalone file may declare the flat markers
`"format": "sono-renderer-notification-config"` and `"schema_version": 1`;
the original markerless document is treated as schema 0. Unknown formats and
future versions disable notifications with a non-fatal diagnostic. Embedded
notification settings follow the containing renderer-config schema.

Example notification config:

```json
{
  "format": "sono-renderer-notification-config",
  "schema_version": 1,
  "enabled": true,
  "native": {
    "enabled": true,
    "events": ["render_completed", "render_failed"]
  },
  "discord": {
    "enabled": true,
    "bot_id": "123456789012345678",
    "bot_token_env": "SONO_RENDERER_DISCORD_BOT_TOKEN",
    "destinations": [
      {
        "enabled": true,
        "guild_id": "123456789012345678",
        "channel_id": "123456789012345679",
        "events": ["render_completed", "render_failed"],
        "templates": {
          "render_completed": {
            "content": "{chart} finished in {render_time}s; {frame_count} frames written."
          }
        }
      }
    ]
  },
  "progress_mode": "disabled",
  "progress_interval_seconds": 60,
  "progress_milestones": [25, 50, 75]
}
```

An event list is a per-backend/per-destination allow-list. Native notifications
and each Discord destination may select different events. Native is enabled by
default for completion and failure only. Discord is disabled by default and
has no default destinations. The global `enabled` switch suppresses every
backend. Progress also requires a non-disabled progress mode and
`render_progress` in the target's event filter.

## Message templates

Native notifications use each event template's `title` and `body`. Discord
uses `content` and may also add an embed using `embed_title`,
`embed_description`, `embed_footer`, and up to 25 `embed_fields` entries (each
has templated `name`, `value`, and `inline` fields). These templates are
independent per Discord destination. The renderer provides the following values when it has
them:

| Value | Meaning |
| --- | --- |
| `{renderer}`, `{renderer_version}` | Renderer name and package version |
| `{engine}`, `{chart}` | Engine and level file names |
| `{output}`, `{output_path}` | Selected output path |
| `{execution_mode}` | `Sono VM / Interpreter` or `Sono-GCC` |
| `{resolution}`, `{width}`, `{height}`, `{FPS}` | Output dimensions and frame rate |
| `{duration}` | Requested duration, or actual output duration after completion |
| `{render_time}`, `{total_time}` | Wall-clock export duration once known |
| `{frame_count}`, `{completed_frames}`, `{progress}` | Available export frame/progress counters |
| `{timestamp}` | Unix UTC seconds when the event was created |
| `{error}` | Error summary on a failed export |
| `{job_id}`, `{job_state}`, `{attempt}`, `{execution_backend}` | Render-job identity, lifecycle state, attempt number, and selected renderer/VM backends |
| `{guild_id}`, `{bot_id}` | Configured Discord identifiers |

`{compilation_time}`, `{cache_status}`, and `{warning}` are accepted for
future/event-specific integrations; they are empty unless an event supplies
them. Missing values become empty text. Unknown or malformed placeholders are
reported in configuration diagnostics and that particular message is skipped.
Use `{{` and `}}` to emit literal braces. Discord messages are JSON-escaped,
length-bounded, and sent with mentions disabled to avoid accidental pings.

## Discord credentials and destinations

Set the configured `bot_token_env` environment variable in the process that
launches Sono-Renderer. The token itself is never stored in the renderer
configuration, passed in a child-process argument, printed in diagnostics, or
included in reports. `bot_id` is optional identity metadata. `guild_id` is
stored per destination for clarity and template use; the numeric `channel_id`
is authoritative for the Discord API route. Channel names are never resolved
or matched. The bot needs permission to view and send messages in each channel.

Discord delivery uses the installed `curl` executable to call the official
channel-message endpoint. It is not a build dependency. If `curl`, the network,
credentials, or Discord are unavailable, a safe diagnostic is written and the
export remains successful. HTTP 429 responses honor Discord's `retry_after`
value (bounded to ten seconds per retry); transient 5xx responses use bounded
exponential backoff. Each channel is limited to one attempted message per
second. The event queue stays bounded while a backend retries.

## Native desktop behavior

The backend uses Windows toast notifications through Windows Runtime,
`notify-send`/the desktop notification service on Linux, and `osascript`
notifications on macOS. These are runtime facilities, not cross-OS build
dependencies. Native commands have a five-second timeout. A missing command or
desktop notification service produces a worker diagnostic and does not stop
rendering.

## Shutdown and verification

The GUI and library APIs do not wait for notification delivery. A CLI render
waits for a worker barrier for at most three seconds after the export has
finished, so the completion event has a chance to leave the process; timing out
there does not undo or delay the finished render. Use a long-running GUI or
embedding process when delivery may take longer.

The test suite uses fake backends and does not need Discord, `curl`, a desktop
session, or internet access. It covers global/backend routing, per-channel
filters, templates, malformed settings, bounded non-blocking queue submission,
backend error/panic isolation, credential absence, rate-limit/backoff policy,
CLI parsing, GUI/shared-config propagation, and JSON persistence.
