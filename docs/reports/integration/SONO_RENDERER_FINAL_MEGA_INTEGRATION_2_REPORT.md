# Sono-Renderer final mega-integration #2

**Date:** 2026-10-07  
**Scope:** Repository coherence, persistence, compatibility, concurrency, and regression audit. This pass adds no product subsystem.  
**Result:** Final-source Windows checks pass. The regular all-target suite passed, and the real project-to-queue-to-FFmpeg integration test passed separately.

## Evidence labels

- **Verified:** directly exercised in this pass.
- **Integration-tested:** exercised through connected renderer subsystems or a real fixture.
- **Unit-tested:** covered by focused test(s).
- **Build-checked:** compiled or statically checked, without interactively exercising the feature.
- **Recorded earlier:** documented in the earlier final integration report, not repeated here.
- **Not tested:** no evidence was collected for this environment or behavior.

## Architecture and ownership

The implemented render flow is coherent around existing shared types:

```text
Project inputs + profile/current settings
                 ↓
       resolved RenderConfig snapshot
                 ↓
       persistent RenderJob / queue
                 ↓
 preflight, cache, Watch VM or Sono-GCC
                 ↓
   CPU/GPU frames, UI, watermark, audio
                 ↓
       FFmpeg output + RenderResult
          ↙            ↓           ↘
 performance      diagnostics     history
                 renderer/job events → notifications
```

| Concept | Authoritative representation | Other views |
|---|---|---|
| Render inputs and settings | `RenderConfig`; project inputs remain separate and resolve into a concrete config | GUI/CLI controls, profile and project files |
| Reusable project | `RenderProject` | GUI/CLI project inspection and validation |
| Queued execution snapshot/state | `RenderJob` and the persistent job manager | Queue UI/CLI; terminal state is copied into history |
| VM/GCC selection | shared execution mode on the resolved job config | Performance and diagnostic records describe what ran |
| Cache identity | Sono-GCC cache manifest and graph/runtime/compiler identity | Cache manager reports and benchmark observations |
| Elapsed-time measurements | `RenderPerformanceRecord` | Diagnostics and history link to the record |
| Analysis and non-timing observations | structured diagnostic record | GUI/CLI details and history link |
| Durable execution attempt | one `RenderHistoryRecord` per terminal attempt | Search, details, export and Render Again |
| Compatibility findings | compatibility analyzer report | project preflight and CLI/GUI reports |
| Lifecycle event | renderer/job event stream | notification backends |

Queue state, measurements, diagnostics, and history intentionally retain separate lifecycles. History and diagnostics may repeat small descriptive snapshots for later inspection, but they do not replace the authoritative queue, performance, or analyzer records. No renderer-to-GUI or VM-to-profile-store dependency was found in the audited flow.

The basic direct-input path remains available: engine, level, resources/music, then render. Project files, profiles, history, notifications, the performance lab, and regression tooling are additive.

## Persistence and recovery inventory

| Store | Current format | Persistence/recovery behavior |
|---|---|---|
| Shared renderer config | Flat JSON with `format` and schema 1; markerless legacy config is schema 0 | Same-directory atomic replacement; old flat configs load, unknown future versions reject |
| Profiles | Schema 1 | Atomic catalog/export writes; import validates; secret values are not exported |
| Projects and recents | Schema 1 | Atomic save; input paths resolve relative to project; validation reports missing inputs; project extensions are retained where supported |
| Persistent jobs | Store version 1 | Lock-protected atomic JSON state; startup reconciliation and separate history sidecar; queue terminal retention is bounded |
| Render history | One schema-1 JSON file per attempt | Atomic per-entry publication; corrupt entries are reported independently; unknown fields are retained; pinned and failed records are protected by retention policy |
| Performance history | Schema 1 records | Atomic per-record writes and bounded retention; queue wait and execution durations are distinct |
| Diagnostics | Schema 1 records | Atomic sidecar writes; corrupt or unsupported records are surfaced without stopping startup |
| Regression references/captures | Schema 1 manifests/reports | Staged/atomic publication; replacing a reference requires an explicit workflow |
| Sono-GCC cache | Manifest v4 plus region metadata | Atomic artifact publication; cache identity includes graph, runtime ABI, compiler, target, and flags |
| UI, watermark and notification config | Schema 1 when marked; legacy markerless files accepted | Future standalone profile schemas reject; notification config failure degrades to disabled with a warning |

**Verified in this pass:** direct `RenderConfig` saves now write the schema marker and atomically replace the old file. A failed validation/save leaves the previous bytes intact. Markerless configs still load, while future versions are rejected. Equivalent future-version checks were added to standalone UI, watermark, and notification configuration readers. This closes the gap between the versioned profile/project/history systems and the direct shared config path.

History sanitization tests cover secret-like token references; profile export tests cover credential omission. No live credential was used. Render output publication uses the existing staged/no-clobber path. Corrupt-history isolation, unsupported schema handling, and retention behavior are covered by tests.

## Integration and stability findings

### Fixed in this pass

1. **FFmpeg first-run installer race:** separate GUI/CLI processes could share the same download archive and staging directory. Managed installation now takes an OS file lock for the full install, rechecks the destination after acquiring it, and times out with a clear error after ten minutes. The operating system releases the lock after process exit. A Windows test verifies lock exclusion across separate file handles and release on drop. A child-process crash/race soak was not run.
2. **Direct config durability/versioning:** saves are now versioned and atomic, with legacy markerless loading and safe future-version rejection, as described above.
3. **Compatibility fixture assertion:** the `m0` test expected an obsolete exact wording for the VM result. It now asserts the structured “Watch VM” assessment and checks that the legacy explanatory string still describes per-function support. The analyzer output itself was not changed.

### Concurrency, cancellation and bounded storage

**Unit-tested:** queue persistence locking and job cancellation/recovery paths; same-key GCC compilation gates; cache entry leases and per-key locks; bounded SFX/GPU caches; cancellation that reaps a running media child; bounded job logs and terminal records. The real project queue render also passed.

History and performance stores have retention controls. GCC disk-cache cleanup is explicit, so cache files can accumulate until the user runs cache management. One remaining growth issue is the job store’s `removed_job_ids` tombstone list: it supports merging stale process state safely but is currently unbounded. Capping it without a stronger generation/compaction protocol could resurrect removed jobs, so it remains documented technical debt.

No long multi-process queue stress, abrupt power-loss injection, resource-exhaustion soak, or concurrent full export stress was run. The FFmpeg lock test verifies the OS lock primitive with separate handles rather than spawning two installer processes.

### VM, GCC, renderer, and performance

The interpreter remains the default correctness path and fallback. Sono-GCC uses the shared execution selection and cache identity, with Watch VM execution for operations/regions that cannot safely run natively. Differential tests cover compiled operations, mixed VM cuts, effects, lazy branches, memory, audio, and display state. Cache hit/miss and fallback are described by the shared performance/diagnostic records.

**Not benchmarked anew in this pass:** no timing assertions were added to correctness tests, and no new VM-vs-GCC speed claim is made. Earlier paired real-render, output-parity, and cold/warm timing results are in [the prior final integration report](SONO_RENDERER_FINAL_INTEGRATION_REPORT.md) and [the Sono-GCC performance report](../benchmarks/SONO_GCC_PERFORMANCE_REPORT.md); those are recorded-earlier evidence, not fresh measurements here.

## Frontends and backward compatibility

**Build-checked and test-covered:** the main GUI and CLI compile against shared render/project/profile/job/history types. CLI parser tests cover existing direct render flags alongside additive configuration, performance, regression, job, and history commands. GUI tests cover shared config/profile application, job workflow state, missing-input repair, cancellation, and background compiler state. The all-target run includes the example targets.

The analyzer’s fixture test now checks structured compatibility status rather than brittle prose. The compatibility CLI and the full paired VM/GCC media export were not manually rerun during this pass; earlier direct CLI/export evidence is labeled above as recorded earlier. No exhaustive manual audit of every GUI control and CLI option permutation was performed.

## Validation performed

**Verified on this Windows machine (`x86_64-pc-windows-msvc`, Rust/Cargo 1.99):**

- `cargo fmt --all --check` — passed.
- `git diff --check` — passed (Git emitted line-ending conversion warnings for the existing dirty working tree).
- `cargo check --offline --all-targets` — passed.
- `cargo test --offline --all-targets` — **386 passed, 0 failed, 4 ignored**: 252 library tests, 21 CLI tests, 3 `arctan2` tests, 101 `m0` integration tests, and 9 Watch scheduler tests.
- `cargo test --offline --lib saved_project_can_queue_and_render_a_real_fixture -- --ignored --nocapture` — **1 passed**. This saved and reopened a project, resolved inputs, queued a new job, rendered the real Yukopi fixture through the renderer and FFmpeg, and verified a nonempty MP4. The test used temporary project/job/output and test performance stores.
- `cargo clippy --offline --all-targets` — exited successfully with warnings. Existing warnings include deprecated `fetch_update`, unused helpers, and style/complexity suggestions; this pass did not perform unrelated cleanup.
- `cargo build --offline --release` — passed on the final source state.

Four tests remain ignored by the regular suite: the private DISPLAYHOLIC dynamic-stage trace, DISPLAYHOLIC MV alpha audit, Horizon SDB particle ledger, and the project queue-render integration test (the latter was run separately above). The three private-corpus diagnostics were not rerun here because they write retained forensic artifacts and two require a private absolute-path media fixture. The previous integration report records their earlier guarded run; that evidence is not counted as a fresh result for this pass.

## Platform, security and production limits

- Only the Windows MSVC target is installed and build-checked. Linux/macOS builds and native-notification behavior were not verified.
- GUI compilation and headless logic tests passed; no interactive desktop click-through was available in this pass.
- No live Discord/OS notification delivery, sanitizer/Miri run, fuzz pass, crash-injection campaign, or extended resource/concurrency soak was performed.
- Basic path/schema/security behaviors have targeted tests, but this is not a penetration test. No live secrets were used.
- No new performance sample was collected, so this report does not make a current speedup/regression claim.
- Existing Clippy warnings and the unbounded job-removal tombstone list remain open maintenance items.

## Reproduction

```powershell
cargo fmt --all --check
git diff --check
cargo check --offline --all-targets
cargo test --offline --all-targets
cargo test --offline --lib saved_project_can_queue_and_render_a_real_fixture -- --ignored --nocapture
cargo clippy --offline --all-targets
cargo build --offline --release
```

Run the three remaining ignored forensic tests only when their private inputs are available and after preserving existing `artifacts/dynamic-stage`, `artifacts/mv-alpha`, and `artifacts/sdb-particle` contents.
