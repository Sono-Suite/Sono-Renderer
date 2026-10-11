# Reproducible render manifests

A render manifest is a portable record of a resolved render attempt. It is
exported from persistent render history and describes the inputs and effective
configuration that actually ran. It is distinct from an editable project, a
profile, a queue job, and the Sono-GCC cache.

## What it records

The `sono-render-manifest` JSON schema is version 1. It contains the renderer
and declared runtime target, requested and effective Watch backend, target
platform, relevant Sono-GCC identity/cache state, history/job/profile
provenance, the sanitized effective `RenderConfig`, output metadata, and
available render references. Each file input has a stable config-slot ID, a
role, active/inactive state, a path hint, byte size, and SHA-256 content
identity when the file was present at export time.

Active assets include the engine, resources, level, enabled music/video,
configured resource overrides, custom gameplay UI events/assets, and enabled
watermark assets. Inactive configured assets remain visible as references but
are not hashed or required for this render. Render outputs are not embedded.
Notification delivery is disabled in the exported snapshot, and job-manager
scheduling settings are replaced with defaults; credentials and local queue
policy are not portable render inputs.

The manifest's canonical SHA-256 covers the content identities, normalized
effective configuration, renderer/backend identity, output metadata, stored
references, and unknown fields. It excludes path hints, output location,
creation/history/job/profile provenance, notification configuration, and
queue scheduling. Moving a manifest and its assets therefore does not change
its identity. The per-file SHA-256 detects content changes even when size and
timestamps are preserved. Hashing is streamed and verifies file metadata did
not change during the read.

Existing RGB-sequence and SFX references are retained using their existing
SHA-1 algorithms and are labeled as such. They are comparison evidence, not
security identities. The SFX PCM reference describes the renderer's stored
pre-AAC SFX mix; it is not a hash of the final MP4's complete audio track.

The renderer build commit and Sono-GCC codegen/runtime identity come from the
authoritative performance record associated with the historical attempt.
They are not sampled from the machine that exports an older history entry. If
that linked record has expired or predates these fields, the manifest reports
the identity as unknown; it does not invent one.

## Reproducibility levels

The verification report states per-component status and does not collapse all
evidence into one green/red result.

| Level | Current evidence |
|---|---|
| 1. Resolved inputs and configuration | `verify` can establish matching active input bytes, effective config, renderer version/recorded commit, semantic target, and recorded requested backend identity. |
| 2. Watch/display-list/event behavior | Not captured in ordinary history manifests. Use an explicit Regression Lab capture/reference for these layers. |
| 3. Raw rendered output | Manifest diff can compare the stored aggregate raw RGB sequence and SFX event/PCM references when both manifests contain them. Audio coverage is partial: the PCM hash is SFX-only and does not include the full BGM track. |
| 4. Encoded media bytes | Not guaranteed or verified. The report marks final encoded-byte identity as not applicable. |

Matching manifest inputs establish Level 1 ingredients, not successful output
equivalence by themselves. A matching RGB reference is evidence for the raw
video sequence. This manifest does not currently import a layered Regression
Lab reference bundle, so it cannot claim runtime/display-list parity from that
system. Hashes detect changed content; they do not attest who created or
signed a manifest.

## Paths, repair, and reproduction

On export, paths beneath the manifest directory are written as relative hints;
other paths remain absolute hints. On load, relative paths are resolved from
the manifest's directory. A moved manifest can therefore use assets moved
with it. Missing or changed files are reported by their stable input IDs and
roles. The renderer does not guess replacements from names. Supply an explicit
replacement for each slot with repeated `--asset ID=FILE` arguments or select
the asset in the GUI.

Reproduction requires a new output path that does not already exist. It
verifies all active file hashes, validates the effective config, and checks
that the recorded requested backend is available before submitting a new job.
The output collision policy of the ordinary job queue remains authoritative.
The new attempt links to its source history/job and manifest SHA-256. Its
result is a new history entry; the old attempt is never changed.

Verification distinguishes input/configuration mismatch from environment
differences. Renderer-version and declared runtime-target differences affect
the semantic reproduction assessment. Operating-system and architecture
differences are reported as environmental context. Sono-GCC exact-backend
reproduction currently requires the supported Windows native loading path and
an available `rustc` toolchain; the manifest path refuses to submit a silent
VM substitute when that backend is unavailable. Runtime fallback within a
supported render remains recorded by the new history outcome.

This is reproducibility evidence, not a promise of identical encoded MP4 bytes
on every machine. The manifest can establish matching resolved file content,
configuration, and requested backend; stored raw RGB and SFX references allow
stronger output comparisons where available. Encoder/platform behavior and
the completeness of renderer/client semantic validation remain separate
considerations.

## Schema and safety

Manifests are limited to 16 MiB and 10,000 input references. The loader checks
format, supported schema version, unique bounded IDs, SHA-256 syntax, and the
canonical manifest digest before use. Unknown fields in schema 1 are retained
for inspection and diffing. Because their meaning is unknown to this version,
they prevent exact reproduction. Unsupported future schema versions are
rejected with a clear error instead of being silently downgraded.

Export uses a same-directory temporary file, flushes it, and publishes with
no-clobber semantics. Existing manifests are never overwritten. A failed
write leaves the previous manifest intact. A manifest only names input and
output paths; opening it does not execute host code. Reproduction writes only
to the explicit new output path after the ordinary queue validates it.

Manifest files are user-managed small JSON records. They have no independent
automatic retention policy; render history and large output/artifact
retention remain owned by their existing stores.

## CLI

```text
renderer manifest export <history-id> render.json
renderer manifest inspect render.json
renderer manifest inspect render.json --json
renderer manifest verify render.json
renderer manifest verify render.json --asset engine=C:/charts/engine.zip --json
renderer manifest diff old.json new.json
renderer manifest reproduce render.json --output exact-rerender.mp4 --enqueue-only
renderer manifest reproduce render.json --asset level=C:/charts/chart.sus --output rerender.mp4
```

`export` reads the configured history store (or accepts `--history-store`).
`verify` hashes the manifest's active inputs, or the explicitly repaired
paths. `diff` categorizes input, configuration, presentation, output,
backend/environment, and provenance differences. `reproduce` creates a fresh
queue job and can either leave it queued or wait for completion. Use
`renderer manifest <command> --help` for the full option list.

## GUI

The Render history tab includes a manifest inspector. Open a manifest to view
its verification report, inspect the effective configuration, select
replacement files for missing assets, compare it with a selected history
attempt, export a manifest from history, and submit a reproduction. Hashing
and config reconstruction run on a background worker; job submission uses the
desktop's long-lived render queue manager.

The GUI logic is build-checked. File-dialog and interactive desktop behavior
depends on the host display session and is not claimed as automated visual
validation.

## Validation coverage and limits

Unit tests cover SHA-256 standard vectors and streaming, manifest round-trip,
secret/scheduler sanitization, canonical identity across moved path hints,
changed/missing inputs, unknown-field preservation, foreign-platform path
hints, and categorized diffs. CLI parsing tests cover command shapes and
ambiguous asset repair. A real renderer integration test exports a manifest
from a completed fixture render, verifies its actual source files, creates a
new queued job using the recorded effective snapshot, renders it, and checks
the new history lineage and RGB/SFX references.

The local SHA-256 implementation is used because the offline dependency cache
did not contain an available SHA-256 crate. Its implementation is isolated
and checked against published standard test vectors; it has not undergone an
independent cryptographic audit.
