# Sono-Renderer Follow-up #9 — Notification system

## Implemented

- Added a generic renderer event and notification backend boundary in
  `src/notifications.rs`. The current reliable export events are
  `render_started`, `render_progress`, `render_completed`, and `render_failed`.
- `RenderConfig::render_video`, `render_video_with_progress`, and
  `render_video_controlled` emit those lifecycle events using actual
  configuration, progress callback, output report, and error data. Internal
  engine loading/compilation/preparation phases are not reported as separate
  events because the export API does not currently expose stable lifecycle
  results for them.
- Notification submission uses a bounded 64-entry `try_send` queue. Native and
  Discord delivery runs on a dedicated worker. Queue pressure drops the new
  auxiliary event without writing to the render thread. Delivery errors and
  backend panics are isolated; a bounded diagnostic buffer feeds the GUI's
  existing Export log and CLI diagnostics.
- Added independently enabled native and Discord backends, global and
  per-backend/per-destination event filters, multiple Discord destinations,
  configurable per-event templates, Discord message content and embed title,
  description, up to 25 named/value fields, inline field styling, and footer.
- Template values are sourced only from real event/config/report values.
  Missing event-specific values expand to empty text. Malformed/unknown
  placeholders are diagnosed and that notification is skipped. Discord
  content/embed lengths are bounded, aggregate embed text is checked, and
  mention parsing is disabled to prevent accidental pings.
- Added disabled-by-default progress announcements with configurable milestone
  or periodic modes. Progress also has to be selected in a target's event
  filter. Defaults announce render completion/failure through native desktop
  notifications; Discord and progress notifications default off.

## Backends and security

- **Windows:** PowerShell invokes Windows Runtime toast notifications. The
  notification process is capped at five seconds.
- **Linux:** invokes `notify-send` and the desktop notification service, with
  the same timeout.
- **macOS:** invokes `osascript` notifications, with the same timeout.
- **Discord:** uses the installed `curl` executable to POST to the Discord
  channel-message API. `curl` is a runtime requirement for this backend, not a
  Cargo/build dependency. Each channel is limited to one attempted message per
  second. HTTP 429 retries honor `retry_after` up to ten seconds per retry;
  transient 5xx responses use bounded exponential backoff, with three retries
  after the first attempt.
- Bot credentials are read only from the environment-variable name configured
  in `bot_token_env`. The secret itself is not part of `RenderConfig`, process
  arguments, saved JSON, GUI fields, or diagnostics. Invalid values that look
  like a raw token are rejected as environment-variable names without echoing
  them. `bot_id` is optional identity metadata. `guild_id` is retained per
  destination and available to templates; numeric `channel_id` is the
  authoritative API route and channel names are not resolved.

No operating-system or Discord Rust dependency was added. A missing OS command,
desktop service, token, `curl`, or network connection produces a worker
diagnostic and cannot fail rendering. The CLI waits on a worker barrier for at
most three seconds after the export returns so completion notifications can
finish before process exit; GUI/library rendering calls do not wait.

## Configuration surfaces

- Added `notifications` to the shared, serde-defaulted `RenderConfig`. Existing
  settings and old files without that field remain compatible. A malformed
  notification-only object is disabled with a non-secret diagnostic instead
  of invalidating the other renderer settings.
- Added a **Notifications** tab to the desktop renderer GUI. It exposes global
  and backend switches, event filters, Discord bot/environment settings,
  multiple guild/channel destinations, templates, embed fields, and progress
  modes. It uses the existing shared-config load/save controls.
- Added `render-video --notifications-config <JSON>` for a standalone
  `NotificationConfig`, and `--no-notifications` for a one-run global opt-out.
  `default-config`, `validate-config`, and `render-config` use the same
  `RenderConfig.notifications` object. Existing CLI flags are unchanged.
- Added [notification usage and schema documentation](../../NOTIFICATIONS.md),
  and updated the shared [configuration reference](../../USER_CONFIGURATION.md)
  and README.

## Validation and limits

The core suite uses fake backends and requires no Discord account, network,
`curl`, or desktop session. Coverage includes global/backend routing modes,
per-destination filters, event templates and embeds, malformed config,
credential absence and redaction, unavailable native commands, backend
failure/panic isolation, bounded queue submission, progress policy, Discord
rate-limit policy, GUI/shared-config propagation, CLI flags, and persistence.

Live delivery was not exercised. The Windows Runtime toast path compiled on
the current Windows target, but an actual Windows toast service was not
validated. Linux/macOS notification services and builds were not available in
this environment. Discord was intentionally not contacted; its runtime
requires a valid bot token in the configured environment variable, channel
permissions, `curl`, and network access. The Discord guild ID is descriptive
and is not cross-checked against the channel through an additional API call.

The configuration system remains explicit: settings persist through the
existing user-selected JSON load/save flow rather than an automatic settings
store. CLI shutdown is bounded; if a backend takes longer than the three-second
post-export grace period, the CLI reports that delivery may have been
interrupted while preserving the completed export.

## Test results

Final verification on the implemented change:

- `cargo check --offline --all-targets` passed.
- `cargo test --offline --all-targets` passed: 159 library tests, 12 CLI tests,
  3 arctan integration tests, 101 `m0` compatibility tests, and 9 Watch
  scheduler tests passed. Three private-corpus library tests were ignored.
- Focused notifications (15), shared-config persistence, GUI propagation, and
  additive CLI-flag tests passed.
- `cargo fmt --all -- --check` and `git diff --check` passed.

The compiler reported existing unrelated deprecation/dead-code warnings in
`runtime.rs`, `audio_prepass_profile.rs`, `export_progress.rs`, and
`profiling.rs`; this change adds no dependency or warning of its own.
