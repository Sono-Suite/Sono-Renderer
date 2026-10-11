# Persistent render history

Render history is the durable record of render attempts. It is separate from
the job queue, which owns pending and active execution. Terminal queue entries
are copied to history before they become eligible for queue cleanup.

## Storage and recovery

History uses one JSON file per attempt, with a stable ID derived from the job
ID and attempt number. The default directory is next to the persistent job
store as `history/`; set `SONO_RENDERER_HISTORY_DIR` or use
`renderer history --store <directory>` to select a different directory. For a
custom queue path, the adjacent `history/` directory is used unless the
history directory is explicitly overridden. GUI and CLI use the same store.

Each record is written to a temporary file, flushed, and published without
replacing an existing attempt. Pin changes and retention settings use atomic
replacement. A crash during a write therefore leaves the previous readable
records intact; a partial temporary file is ignored. Startup reconciliation
backfills terminal queue records that were not yet recorded before a crash.
The queue marks an attempt history-persisted only after the history file is
durable, then retains at most its latest 100 persisted terminal jobs for queue
inspection. History itself is not held in that queue.

History format `sono-render-history` is schema version 1. Version-zero records
are migrated when read. Unknown fields are retained for forward-compatible
read, pin, and export operations. Unsupported future schemas and malformed
files are reported as per-entry issues; they do not prevent valid entries from
being listed. Keep a backup before manually editing record files.

## Record contents and linked data

An entry describes one execution attempt: job and attempt IDs, project and
profile identity, the resolved effective `RenderConfig` snapshot, renderer
version, OS and architecture, backend, timestamps, input paths and basic file
metadata, output path and size, terminal status, warnings/error/fallback, and
references to the existing authoritative performance and diagnostic records.
It also records retry/rerender lineage and cache/backend state where those
systems reported them. It does not copy render outputs, diagnostic dumps, or
performance artifacts into the history directory. Input identity currently
uses path, file size, and modified time; it does not hash large source files,
so history can detect common changes but cannot prove byte-for-byte input
identity after a source file is replaced while preserving its metadata.

The snapshot is taken from the queued job, not reconstructed from the current
profile. A deleted or changed profile therefore does not change what the
record says actually ran. Notification credentials are not stored: the
configuration contains only an environment-variable name, and a value that
does not look like a valid variable name is replaced with the default
reference. Credentials are read from current secure environment storage when
the new render runs. `Render Again` uses the saved effective snapshot by
default.

Performance and diagnostic records have their own storage and retention. If a
linked record has been removed, history remains readable and indicates that
the detail is unavailable.

## Browsing and retention

The desktop **Render history** tab shows the newest 100 matching attempts and
supports text, status, engine, project, and backend filters. Selecting an
entry shows its inputs, output metadata, effective configuration, associated
diagnostics/performance, warnings, and error. The tab can open or reveal an
existing output, export an entry, pin/unpin, delete, or clear unprotected
entries. Retention controls support an entry limit, optional age limit, and
separate protection for failed and pinned attempts.

Default retention is 1,000 entries with no age limit; failures and pinned
entries are kept. Protected records can make the total exceed the configured
limit. A user-initiated clear preserves failures and pinned attempts. Use the
CLI `clear --include-failures` or turn off the corresponding retention option
only when deleting those records is intended. Large diagnostics and render
outputs remain governed by their own storage policies.

## CLI

Examples:

```text
renderer history list --limit 50 --status failed
renderer history list --search "Studio" --engine "engine.son"
renderer history list --backend gcc --json
renderer history show <history-id>
renderer history rerender <history-id> --enqueue-only
renderer history rerender <history-id> --output rerender.mp4
renderer history pin <history-id>
renderer history delete <history-id>
renderer history clear
renderer history prune --max-entries 500 --max-age-days 90
renderer history export <history-id> history-entry.json
```

Add `--store <directory>` before the history action to choose a history store;
`--queue-store <file>` selects the queue used by `rerender`. `list --json` and
`show --json` provide structured records. `rerender` creates and submits a
new queue job with the saved effective configuration and lineage link. It
checks required inputs and the output path before submission. A missing input
is named and the CLI stops rather than guessing a replacement. If the old
output exists, pass a new path with `--output`; existing output is never
silently overwritten.

## GUI Render Again and repair

Render Again always constructs a new job. If source inputs are still present,
it queues the saved snapshot immediately; if the historical output path is
occupied or its parent is gone, the GUI asks for a new output path. The queue's
normal no-clobber output reservation still applies.

If required inputs have moved, the GUI lists each missing role and accepts a
replacement for that specific role. It can also load a relocated project and
use its resolved relative references for the corresponding missing inputs. It
does not select a candidate from a filename guess. The current profile is not
silently substituted; the history snapshot remains the default source of
configuration.

## Validation

History model tests cover serialization, unknown-field preservation, version
migration/future-version isolation, corrupt-file recovery, sanitization,
retention, and filters. Queue tests cover terminal cancellation persistence,
startup backfill/idempotence, and rerender provenance. CLI tests cover
filter parsing, missing-input/output collision refusal, and submission as a
new queue job. The real renderer integration fixture checks successful VM and
Sono-GCC attempts, a failed input attempt, retry lineage, profile snapshots,
and linked performance/diagnostic data.

The GUI history logic is build- and unit-tested. Interactive browsing, file
dialogs, and input repair require a desktop session and were not exercised by
the automated validation run.
