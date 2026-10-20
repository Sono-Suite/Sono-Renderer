# Sono-Renderer Follow-up #21 — maintainability and architecture freeze

## Evidence labels

- **Verified** means checked against current source or generated structured
  output.
- **Tested** means an executable test or render completed successfully.
- **Build-checked** means Cargo compiled the named targets.
- **Manually inspected** means source/dependency paths were reviewed without
  claiming exhaustive formal analysis.
- **Not tested** means the current environment did not establish the claim.

The worktree contained changes and untracked reports/artifacts from earlier
follow-ups before this pass began. Those files were preserved. No user data,
saved Regression Lab reference, engine, profile, or production output was
overwritten.

## 1. Architecture before cleanup

**Verified / manually inspected.** Sono-Renderer is one Rust package, not a
Cargo workspace. `src/main.rs` owns CLI orchestration and desktop launch;
`src/gui.rs` owns the eframe frontend. Both frontends call the same library
modules. `RenderConfig` is the common request; `RenderProject` and profiles
resolve into it; queued work captures it in `RenderJob`.

The render path is `RenderConfig` → `render.rs` → `FrameSession` and
`WatchFrameStepper` in `offline.rs` → `WatchRuntime` / `WatchVm` → owned
`PreparedFrame`s → CPU/WGPU composition in `gpu_render.rs` → FFmpeg and audio
finalization in `video_export.rs` and adjacent media modules. Sono-GCC wraps
eligible scalar regions; Watch VM remains the semantic oracle and fallback.
Performance and diagnostic records are made at the actual export boundary;
history consumes terminal queue attempts; notifications are auxiliary events.

The full current module map and ownership table are in
[docs/ARCHITECTURE.md](../../ARCHITECTURE.md).

## 2. Major technical-debt findings

**Manually inspected.** Three concrete boundary issues justified edits:

1. `render.rs` imported queue scheduler types from `job_manager.rs`, while the
   manager imports `RenderConfig` to execute work.
2. `render_history.rs` imported live queue types and reached back into the
   manager to find its default store path; the manager also calls history when
   finalizing attempts.
3. Retired event-only audio profiling state and helpers remained after normal
   SFX collection moved into the rendering traversal.

The large frontends (`main.rs` and `gui.rs`) and broad `RenderConfig` remain
large, but are recognizable ownership boundaries today. I did not split them
mechanically.

## 3. Duplicate sources of truth found

**Verified.** Queue and history each had separate implementations of the
default sibling store-path rule. These now call the private `store_paths`
module. The queue-to-history projection remains one mapping from
`RenderJob` to the canonical `RenderHistoryRecord`; only its code location
changed. No second job, performance, diagnostic, compatibility, cache, or
notification model was introduced.

The `tempfile` crate was listed under both normal and dev dependencies. It is
needed by production persistence and therefore remains a normal dependency;
the redundant dev-dependency declaration was removed.

## 4. Dead code removed or restricted

**Verified / build-checked.** Removed the unreachable private
`ProfileCollector::record_event_only` implementation and its event-only
counters. Existing profile-output slots still print zero, preserving their
observable text and reflecting that the production event-only pass is gone.
The test-only checkpoint accumulator and the test-only detailed GCC compile
helper are now compiled only for tests. `Memory::len` is also test-only because
only that checkpoint accumulator uses it. None of those changes alter normal
Watch or render behavior.

`ProgressTracker::pulse` is still public and remains because deleting a public
method would be an API change; its only current caller is a test helper. The
existing deprecated atomic spelling remains because this package declares no
MSRV with which to judge whether the suggested replacement is compatible.

## 5. Duplicated logic consolidated

**Verified.** Queue and history store defaults now share `store_paths.rs`.
Scheduler settings have a single internal definition in `job_config.rs`.
History serialization/validation remains owned by `render_history.rs`, while
the queue lifecycle performs the projection from its live job. Existing
public Rust paths for `JobPriority`, `JobManagerConfig`,
`RenderHistoryRecord::from_job`, and `RenderHistoryStore::save_attempt` remain
available; the scheduler types are re-exported from `job_manager`.

No broad validator, path, serializer, clock, or error utility was added.

## 6. Dependency cycles removed

**Build-checked / verified.** The queue/history type-and-path cycle is
removed: `render_history.rs` no longer imports `job_manager.rs`; the manager
projects an attempt into history, and both use the neutral path module.
The renderer/configuration-to-manager type cycle is removed: `render.rs`
uses `job_config::JobManagerConfig`, and the manager re-exports the same
public type.

There is still deliberate collaboration among rendering, telemetry, and
diagnostics. `RenderConfig` remains a broad shared configuration type, and
`render.rs` persists the authoritative telemetry/diagnostic products. Breaking
those connections would require a larger request/configuration redesign, so
they are documented rather than expanded in this pass.

## 7. Error-model findings

**Manually inspected; no broad error rewrite.** Loaders and stores generally
propagate `anyhow::Result` with file/operation context. Lifecycle outcomes,
diagnostic findings, and failure kinds use enums where callers branch on
meaning. The job queue records technical cause chains in diagnostics and
logs. Notification submission/delivery is intentionally best-effort and
cannot fail rendering.

The broader crate still has some context-light internal errors and panic-based
assumptions for programmer-owned invariants. No panic on an external-input
path was demonstrated by this audit, so I did not perform a sweeping
string-to-error-type conversion. A future API review should decide which
publicly exposed library errors need stable typed variants.

## 8. Concurrency and locking findings

**Manually inspected.** The job manager owns worker threads, queue state, and
cancellation controls. The Watch runtime owns a per-runtime bounded callback
pool and commits its results in callback order. The export pipeline owns its
bounded frame handoff and FFmpeg child. GUI export/precompile threads are
frontend-owned. Notifications use one bounded worker and nonblocking
submission. GCC process-cache lookup is short; same-key compile gates and
disk-entry locks serialize artifact work.

The queue state mutex still covers synchronous queue serialization and the
on-disk store lock. Render, compile, history publication, and network
notification delivery run after the state guard is released. This is a known
contention point, not changed without evidence that a lock-scope rewrite is
safe. There is no universal lock-order contract; new code should avoid nested
application locks and must not carry the queue guard into render/compiler or
network work.

## 9. Persistence ownership

**Verified.** Each persistent format has one reader/validator/migrator/writer
owner: shared config (`render.rs`), project and recents (`project.rs`), full
profiles (`config_profiles.rs`), gameplay UI (`render_ui.rs`), watermark
(`watermark.rs`), queue (`job_manager.rs`), attempt history
(`render_history.rs`), performance (`performance.rs`), diagnostics
(`render_diagnostics.rs`), Regression Lab (`regression.rs`), GCC manifest
(`sono_gcc.rs`), and notification settings (`notifications.rs`). The current
schemas and save behavior are recorded in the architecture guide. No schema
version or migration behavior changed in this pass.

## 10. Naming and terminology

**Verified / documented.** The architecture guide names the current canonical
terms: `RenderProject` (inputs), `RenderConfig` (resolved request),
`RenderJob` (queue snapshot/lifecycle), `RenderHistoryRecord` (attempt),
`RenderPerformanceRecord`, `RenderDiagnosticRecord`, and
`WatchExecutionMode`. No public rename was necessary. CLI labels and
serialized names were preserved.

## 11. Test architecture and intent

**Tested.** Tests remain grouped by subsystem and actual integration
boundaries: module-local semantic tests, `tests/arctan2.rs`,
`tests/watch_scheduler.rs`, and the real-fixture `tests/m0.rs`; opt-in
Regression Lab and performance flows remain separate from normal timing
assertions. The post-change test command compiled the GUI-bearing package and
examples as well as the library and binary.

The architecture guide names representative invariants protected by tests,
including VM `Draw(-1)` behavior, watermark master-off, exact GCC cache
identity, and history rerender submission. I did not merge or relocate test
modules. Four existing opt-in forensic/corpus tests remain ignored and are
counted below; they were not needed to establish this source-only cleanup.

## 12. Documentation added or updated

**Verified.** Added [docs/ARCHITECTURE.md](../../ARCHITECTURE.md) with the
current subsystem map, execution diagrams, authoritative data types,
persistence schema ownership, thread/lock boundaries, unsafe/platform notes,
test groups, invariants, and proposed freeze boundary. README links to the
guide and this report. Pre-cleanup measurements and the first regression
capture are recorded in [BASELINE.md](../../../artifacts/architecture-freeze-followup-21/BASELINE.md).

## 13. Unsafe and FFI audit

**Manually inspected.** Unsafe Rust is limited to the generated Sono-GCC
Windows DLL call/load/unload boundary, platform process-liveness checks in
`job_manager.rs`, and Windows `FreeConsole` in `gui.rs`. The generated GCC
entry checks argument length and nullness before creating a slice;
`NativeLibrary` owns the handle lifetime. WGPU and FFmpeg use safe Rust APIs
and subprocess boundaries respectively. No unsafe block was rewritten.

## 14. Platform boundary audit

**Build-checked on Windows only.** Executable naming/provisioning,
notifications, PID checks, console behavior, and file reveal are placed in
their owning adapter modules. This pass's Cargo check/build/test environment
was Windows x86_64 MSVC. Linux and macOS builds, notification backends, and
runtime behavior remain **not tested** here.

## 15. Performance before and after

**Measured, but not an apples-to-apples performance conclusion.** The same
one-second Larp segment, config fingerprint, backend settings, 1 warm-up, and
3 measured runs were used before and after. The after runs reused the same
isolated GCC cache. Baseline and post records are in
`artifacts/architecture-freeze-followup-21/`.

| Backend | Baseline median | Post median, run 1 | Post median, repeat |
|---|---:|---:|---:|
| Sono VM | 3504 ms | 5246 ms | 5270 ms |
| Sono-GCC warm | 3296 ms | 4760 ms | 5051 ms |

Render preparation and the rendering phase rose by roughly 40–60%; other
phases varied (Sono-GCC audio encoding was nearly flat). Frame/callback/
evaluation counts stayed identical. A short system CPU sample during the post
measurements showed 44–71% busy time. A processor-clock query was denied by
the environment, and the Performance Laboratory does not record transient
host load. The slower measurements therefore cannot be treated as a code
regression or as a clean no-regression result. **The cause remains
unverified.** I did not dismiss the change: the representative performance
comparison is unresolved and should be repeated on an idle, controlled host
before release.

## 16. Visual/semantic regression results

**Tested.** The canonical Next Rush Regression Lab fixture was run with the
saved VM/GCC reference, 1280x720 and 1024x768, and matching UI/watermark
profiles. The aggregate command returns exit 1 because all eight checkpoint
timings differ from the older reference and four GCC checkpoints also report
the old region/cut counter difference. This is historical reference drift, not
a clean aggregate pass.

For this cleanup specifically, the pre-change and post-change captures were
compared directly: all eight PNG SHA-256 hashes match, and all 64 current
semantic/pixel layer hashes match. Both captures report zero changed pixels.
Both VM/GCC checkpoints match. The pre/post artifacts are retained under
`artifacts/architecture-freeze-followup-21/regression/`; the preserved
reference was not replaced.

## 17. Real-engine validation and direct workflow

**Tested.** The direct CLI workflow `engine + resources + level + music →
MP4` ran after cleanup without a project, profile, queue, history, or
notification service. It rendered one second at 12 FPS/640x360, CPU/VM,
particles, SFX, and BGM enabled, with `--no-notifications`. FFprobe validated
12 H.264 frames and one second of AAC. The output is 92,093 bytes and its
SHA-256 is identical to the pre-cleanup direct render:
`cbb89344e5feae5463c2841c84b4f9d344fd62c13c31ac6556c4f7c093e6133e`.
Watch traversals remained one; the SFX event sequence remained 93 scheduled
effects, three loop starts, and three stops.

The full test suite also exercised supplied Next Sekai/Next Rush assets,
VM/GCC differential operations, CPU/WGPU composition, audio export, UI,
particles, and history/job flows.

## 18. Remaining intentional complexity

**Verified / manually inspected.** One Watch runtime supports both
sequential and bounded parallel callbacks while preserving commit order. The
export producer/consumer pipeline overlaps work and must retain explicit
timing semantics. Sono-GCC's native regions and VM cuts are required for its
hybrid model. Per-attempt, per-record stores isolate corruption. Separate
project, profile, job, performance, diagnostic, history, and notification
models represent distinct user concepts and remain intentionally separate.

## 19. Remaining technical debt and limitations

- `gui.rs` and `main.rs` remain large frontend/command modules; any future
  extraction should follow page/command ownership, not arbitrary line counts.
- `RenderConfig` still groups many render and scheduler preferences and the
  renderer module coordinates output plus telemetry persistence.
- Queue mutations synchronously persist under the state lock.
- `JobState` is an enum with manager-side transition guards rather than a
  compile-time typestate machine; serialized/public job fields remain broad.
- The public Rust module surface is wider than a formally versioned stable API
  contract. The freeze below is a proposed compatibility boundary, not a
  semver guarantee already published.
- Clippy exits successfully but still reports roughly 140 library test-target
  warnings plus binary/integration warnings. Most are unrelated style or
  argument-count suggestions; they were not mass-applied.
- `cargo machete` is unavailable. Duplicate Cargo tree entries are primarily
  transitive through GUI/GPU stacks; no dependency upgrade/removal was
  justified beyond removing the redundant `tempfile` dev declaration.
- Performance before/after is unresolved on this busy host; Linux/macOS and
  interactive GUI behavior are untested.

## 20. Recommended architecture freeze boundary

Freeze these for the next stable release:

1. Direct CLI render inputs and their existing behavior.
2. `RenderConfig` field meanings/defaults and profile/project resolution into
   an immutable submitted-job snapshot.
3. Project, profile, queue, history, performance, diagnostic, regression, UI,
   watermark, notification, and GCC-cache schema versions and migration
   behavior documented in the architecture guide.
4. `RenderJobManager` as active lifecycle owner; history remains a durable
   record of attempts and Render Again submits a new job.
5. `RenderPerformanceRecord` as timing authority; diagnostics and history
   reference it rather than starting duplicate clocks.
6. `WatchVm` semantics, the single `WatchRuntime` lifecycle path, and
   Sono-GCC's VM-parity/fallback boundary.
7. The twelve semantic invariants listed in
   [docs/ARCHITECTURE.md](../../ARCHITECTURE.md).

Future features may extend these surfaces, but should not introduce a second
Watch runtime, duplicate a persistent schema owner, or change these contracts
without an explicit migration and regression plan.

## Validation record

**Tested before any source edits:** serial and default-parallel
`cargo test --offline --all-targets` each passed 386 tests (four ignored).
One earlier parallel attempt aborted once with Windows
`STATUS_HEAP_CORRUPTION` (`0xc0000374`); it did not recur in either completed
baseline run or the post-change parallel run. Cause unknown.

**Tested after cleanup:** `cargo test --offline --all-targets` passed 386,
failed 0, ignored 4. **Build-checked:** `cargo check --offline --all-targets`,
`cargo clippy --offline --all-targets` (warnings remain), and
`cargo build --offline --release` exited successfully. The direct render,
matched performance benchmark, and Regression Lab runs are described above.
`cargo fmt --all -- --check` and `git diff --check` passed; the latter printed
only Git's existing LF-to-CRLF working-copy notices. `cargo check` was repeated
after removing the duplicate dev-dependency declaration. The release build and
full test suite preceded that manifest-only deduplication; all-target check
after it passed.
