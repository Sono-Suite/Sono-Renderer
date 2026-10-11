# Sono-Renderer Follow-up #22 — Render Manifest Report

## Result

Added a portable, versioned render manifest by composing the existing render
history, performance records, `RenderConfig`, job queue, renderer, and GUI/CLI.
The manifest describes one resolved historical execution. It does not replace
projects, profiles, history, the active queue, cache, diagnostics, or the
Regression Lab.

## Architecture and schema

`src/render_manifest.rs` owns manifest serialization, validation,
canonicalization, input verification, diffing, and construction of a safe
reproduction config. `RenderManifest::from_history_with_performance` reads the
effective config and provenance from one history record and compiler/build
identity from that attempt's linked authoritative performance record. Missing
or expired telemetry remains unknown; an exporter never claims its current
compiler was used by an older render.

Schema `sono-render-manifest` version 1 is a bounded JSON document (16 MiB,
10,000 input slots). It contains the renderer version/commit when available,
declared Sonolus target, requested/effective Watch backend, platform,
Sono-GCC codegen/runtime ABI and cache state when recorded, provenance,
sanitized effective config, output metadata, and stored render references.
Configuration already carries UI/watermark settings, dimensions, FPS,
timeline, audio/export choices, and deterministic seeds. Active file slots
include engine, resource package, level, enabled music/video, resource
overrides, active custom UI events/assets, and enabled watermark assets.

Unknown fields are retained and shown during inspection/diff, but block exact
reproduction because their semantics are unknown. Unsupported future schema
versions are rejected. Current schema-1 records require no migration. The
performance record received additive defaulted compiler/build identity
fields; older schema-1 performance records remain readable with unknown
identities.

## Identity, paths, and safety

Each active input is hashed from file content with SHA-256 and byte length;
the file is streamed and checked for metadata changes during hashing. The
canonical manifest SHA-256 includes normalized config/content identity,
renderer/backend identity, output metadata, stored references, and unknown
fields. It omits local input path hints, output location, creation and
history/job/profile provenance, notifications, and local queue scheduling.
Absolute paths therefore do not define identity. Existing RGB sequence, SFX
event, and SFX PCM hashes retain their existing SHA-1 representation and are
comparison references rather than security identities.

When saved, paths beneath the manifest directory become relative hints; other
paths remain absolute hints. Loading resolves relative hints from the
manifest's parent. Missing or changed assets are reported by input ID and role.
Users explicitly repair each slot with `--asset ID=FILE` or the GUI file
picker. No filename guessing or recursive drive scan is used. Assets and
outputs are referenced, never embedded. Notifications are disabled and job
scheduler settings are reset in the exported config; secrets are not
included. Hashes check content integrity, not authorship.

Manifest saves use a same-directory temporary file, flush, and no-clobber
publish. Existing files are not overwritten. The writer enforces size and
schema bounds before publishing.

## Relationships and behavior

- **Project/profile:** A project is editable input intent; profile identity is
  provenance. The manifest stores the resolved historical `RenderConfig`, so
  later project/profile edits do not rewrite it.
- **Job/history:** Reproduction builds a new validated config and submits a new
  ordinary `RenderJob`. It does not resurrect the source job. The resulting
  history entry stores `reproduced_from_manifest_sha256` and source lineage.
  Existing output collision checks remain active.
- **VM/GCC/cache:** The captured requested backend is retained. A GCC
  reproduction checks platform/toolchain and recorded compiler/runtime
  identity; it does not silently switch to VM. Missing cache is not a manifest
  failure because the ordinary renderer can rebuild it. VM remains the
  semantic oracle.
- **Performance/diagnostics:** Compiler/build identity comes from the existing
  performance record. New performance and diagnostic records are created by
  the normal queue/renderer path. The manifest adds no stopwatch or separate
  diagnostics store.
- **Comparison/regression:** Manifest diff categorizes input content,
  configuration, presentation, output encoding, backend/environment, and
  provenance. It compares the existing raw RGB/SFX references. It does not
  import a layered Regression Lab reference bundle; runtime, display-list,
  event-stream, and watermark-pose comparison still use the explicit
  Regression Lab workflow.

The Render history tab can export a manifest, open/inspect/verify one, repair
assets, compare a selected history attempt, inspect the effective config, and
reproduce. Input hashing and config reconstruction run in a background
worker. Submission uses the GUI's existing long-lived job manager and copies
local scheduler settings from that manager. The CLI provides
`renderer manifest export|inspect|verify|diff|reproduce`; inspection,
verification, and diff support JSON output.

## Reproducibility contract

1. **Resolved inputs/configuration:** Verified when active content identities,
   config validity, renderer version/recorded commit, semantic target, and
   required backend identity match.
2. **Watch/display-list/event behavior:** Not established by ordinary history
   manifests. Use layered Regression Lab captures.
3. **Raw rendered output:** Manifests can compare the existing aggregate RGB
   sequence and SFX event/PCM references. PCM evidence covers SFX only, not
   the full BGM mix. A matching RGB hash is raw-video evidence.
4. **Encoded media bytes:** Not guaranteed. The structured verification
   report marks final-byte identity as not applicable.

The verifier reports a status per component, including missing/mismatched
inputs, renderer/build/backend differences, unknown metadata, and the
final-byte limitation. Cross-OS/architecture differences are environmental;
they are not treated as an input-content mismatch. A matching manifest does
not prove that another renderer or Sonolus client is semantically correct.

## Validation

### Verified / tested

- Initial pre-change baseline: `cargo test --offline --all-targets` passed
  (386 passed, 4 ignored; before Follow-up #22 changes).
- `cargo fmt --all` and `cargo check --offline --all-targets` passed. The
  check includes the desktop GUI binary and all targets.
- Manifest unit tests pass for serialization, secret/scheduler sanitization,
  canonical path independence, moved-input override, modified/missing content,
  unknown-field handling, non-native path hints, and categorized diff.
- SHA-256 implementation passes standard empty, `abc`, and multi-block
  vectors, including incremental updates.
- Performance tests pass for capturing GCC identity only in GCC mode and
  reading older schema-1 records with the new fields absent.
- Manifest CLI parser tests pass for export/inspect/verify/diff/reproduce
  shapes and ambiguous asset repair rejection.
- Isolated real Next Rush renderer integration passes through actual VM and
  GCC jobs, history/performance/diagnostic persistence, manifest export and
  verification, fresh queue submission, rendered output, new history lineage,
  and matching stored RGB/SFX references. The VM profile is changed after
  submission; the reproduced manifest keeps the captured duration.
- Cross-machine path behavior is simulated by copying the same active inputs
  under a different temporary root, explicitly supplying those paths, and
  verifying matching content and a valid reproduction config.
- The GUI path is build-checked. Interactive file dialogs and rendering were
  not exercised in a desktop session.

### Measured hashing cost

The real integration fixture contained 3 active files totaling 15.64 MiB.
In the debug test build, manifest generation took 1,557.7 ms and verification
took 1,528.9 ms. These are single-run measurements, not a benchmark
distribution or release-build estimate. GUI hashing is asynchronous. The
format prioritizes hashing source bytes over trusting filename/size/mtime.

### Final validation

- `cargo fmt --all -- --check`: passed.
- `cargo check --offline --all-targets`: passed, including the GUI binary.
- `cargo test --offline --all-targets -- --test-threads=1`: passed, with 397
  tests passed and 4 ignored (library 261, CLI 23, existing integration target
  3, `m0` 101, Watch scheduler 9). The library suite took 735 seconds and
  `m0` took 94 seconds.
- A later small representation-only cleanup boxed GUI worker payloads and
  derived two default implementations. After that cleanup, all-target check,
  all five manifest unit tests, Clippy, and release build passed. The full
  suite was not rerun after this allocation/layout-only cleanup.
- `cargo clippy --offline --all-targets`: succeeded. It reports the existing
  repository lint set plus two `chunks_exact_to_as_chunks` advisories in the
  new SHA-256 implementation; `chunks_exact` was retained as the more broadly
  stable slice API.
- `cargo build --offline --release`: passed. Release CLI help smoke checks
  passed for `manifest` and `manifest reproduce`.
- The final parallel all-target attempt aborted inside the Windows Rust test
  process with `0xc0000409 STATUS_STACK_BUFFER_OVERRUN` and no Rust test
  failure report. The subsequent complete serial run passed. The isolated
  media-time test, manifest tests, and real reproduction test also passed.
- `git diff --check`: passed. The repository has many pre-existing modified
  files from earlier integration work; this report does not attribute those
  unrelated edits to Follow-up #22.

No release-build hash-time measurement is claimed.

## Known limits

- GCC re-execution through the normal queue is supported, and compiler
  identity capture is integration-checked, but this fixture reproduced the
  VM manifest. A separate end-to-end GCC-manifest reproduction was not run.
- Cache deletion/cold GCC reproduction and different-OS execution were not
  exercised. Manifests do not depend on cache paths or cache contents.
- Relocated inputs under a different local root were verified on Windows.
  The `cfg(not(windows))` fixture for loading a foreign Windows path hint was
  not executed on this Windows host, and no non-Windows runtime was available.
- The manifest currently carries aggregate raw RGB/SFX references, not a
  layered Regression Lab bundle. It cannot independently establish
  Level-2 Watch/display-list parity.
- The local SHA-256 implementation was necessary because no SHA-256 crate was
  available in the offline dependency cache. It passes standard vectors but
  has not had an independent cryptographic audit.
- Atomic no-clobber and round-trip behavior are tested, but power-loss fault
  injection and directory-sync behavior were not tested.

## Architecture boundary

Keep `render_manifest.rs` as the portable snapshot adapter. Future changes
should continue to read resolved state from history/configuration and submit
through the existing queue. Any new raw semantic reference should be owned by
the Regression Lab or authoritative render telemetry and linked here, rather
than creating a second evaluator, renderer, cache, job lifecycle, or timing
model.
