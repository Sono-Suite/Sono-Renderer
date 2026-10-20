# Sono-Renderer Post-Integration Follow-up #7

## Persistent render history / Render Again

Implemented a durable history record for each completed, failed, or cancelled
render attempt. History is a consumer of the existing job, profile,
performance, diagnostic, and project systems; it does not replace the queue or
create a second performance/diagnostic model.

## Architecture and persistence

`src/render_history.rs` defines the versioned `RenderHistoryRecord` model and
store. Each attempt has its own atomic JSON file, identified by job ID and
attempt number. This bounds the failure domain to one record and avoids a
single ever-growing history document. `retention.json` has a separate schema
version. Writes use a flushed temporary file and no-clobber publish; pin and
retention updates use atomic replacement. Bad or future-version files appear
as per-entry issues while valid records remain browseable. Schema version zero
is migrated to version one on read, and unknown flattened fields survive
read/pin/export.

Terminal jobs are saved before the queue marks `history_persisted`. Startup
reconciliation backfills a terminal queue entry if a crash occurred between
those operations. Persisted terminal jobs are then pruned from the active
queue, which keeps at most the latest 100 terminal entries for queue
inspection. History records are immutable per attempt apart from pin state.

## Configuration, secrets, and lineage

The history record snapshots the effective `RenderConfig` held by the actual
queued job, plus profile identity, project path, renderer/build environment,
input identities, output metadata, status, timestamps, warnings, final error,
cache/backend fallback details, and retry/rerender relationships. Profile
changes or deletion do not rewrite this snapshot. Credentials are not stored:
the config has only a token environment-variable reference; malformed values
that resemble a token are replaced before persistence. A new attempt resolves
credentials from current environment storage.

Engine, level, resources, optional music/MV/overrides, and output paths are
references and metadata only. Rendered media and large diagnostic artifacts
are not copied into history. Performance and structured diagnostic details
are linked by their existing record IDs and can expire independently.
Input identity records path, size, and modification time, not a content hash;
it will flag common changes but cannot prove byte-for-byte identity if a file
is replaced while preserving both metadata values.

## Retention and recovery

Default retention is 1,000 entries, with no age cutoff; failed and pinned
records are protected. Protected entries may make the count exceed the limit.
The GUI and CLI can set max entries, optional age, and failure/pin protection;
manual delete, clear, and export are also available. Corrupt history files do
not block application startup or valid history browsing. Corrupt files are
reported, not silently deleted.

## GUI and CLI

The current GUI has a **Render history** tab with search, status/engine/project/
backend filters, newest-first bounded display, entry details, output open and
reveal, config snapshot, diagnostic and performance details, comparison with
another compatible performance record, export, pin, delete, and retention
controls. Render Again reuses the saved effective snapshot and creates a new
queue job. Missing source inputs are identified by role; the GUI allows
role-specific replacement and can resolve references from a relocated
project. Occupied/missing output destinations prompt for a path and normal
queue no-clobber checks still apply.

`renderer history` supports `list`, `show`, `rerender`, `delete`, `pin`,
`unpin`, `clear`, `prune`, and `export`; list/show can emit JSON. CLI rerender
checks inputs and output collisions and refuses to guess missing paths. It
uses the historical configuration, even if the referenced profile has
changed or disappeared. The existing `renderer jobs` commands remain intact.

Detailed CLI examples and storage semantics are in
[docs/RENDER_HISTORY.md](../../RENDER_HISTORY.md).

## Validation

Validation performed:

- `cargo check --bin renderer` passed after the final history-store wiring.
- `cargo test --lib render_history::tests -- --nocapture` passed: 5 tests,
  including corruption isolation, schema handling, secret sanitization, and
  retention.
- `cargo test --lib job_manager::tests -- --nocapture` passed: 10 tests.
- `cargo test --bin renderer history_ -- --nocapture` passed: 3 tests,
  including actual rerender submission to a temporary queue.
- `cargo test --lib gui::tests -- --nocapture` passed earlier: 15 tests.
- The real renderer integration test
  `video_export::tests::single_watch_export_audio_pipeline_offsets_and_failure_cleanup`
  passed. It exercised VM and Sono-GCC output, failed input, retry, separate
  attempt records, historical profile snapshot after profile mutation/removal,
  and performance/diagnostic associations.

The GUI compiled and its UI logic tests passed, but interactive browsing,
output dialogs, and missing-input repair were not run in a desktop session.
Performance and diagnostic payload retention remains owned by those systems;
history will show a missing linked record if it has expired.
