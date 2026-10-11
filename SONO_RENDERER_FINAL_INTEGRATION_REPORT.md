# Sono-Renderer final integration and stability report

**Date:** 2026-10-06
**Scope:** Audit and validate the implemented system; do not add a new product subsystem.
**Result:** The available Windows test and release-build checks pass. This is not a claim of cross-platform or fully production-certified behavior. The only repository changes made during this final pass were documentation corrections and this report; no renderer feature code was changed.

## Evidence labels

- **Verified:** exercised directly in this pass, with observable output or a checked result.
- **Integration-tested:** exercised through multiple real application subsystems or real-engine fixtures.
- **Unit-tested:** covered by focused tests.
- **Build-checked:** compiled or statically checked, without interactive behavior validation.
- **Not tested:** no evidence was collected for the stated environment or behavior.
- **Existing benchmark:** evidence from the repository's earlier documented benchmark pass, not rerun in this final pass.

## 1. Architecture overview

**Inspected.** The main path is `RenderConfig` plus input paths, optionally resolved from a named profile, into either a direct render or a persistent render job. The renderer prepares the engine and level, executes Watch through the interpreter or the optional Sono-GCC hybrid, produces frames/audio, applies gameplay UI and watermark layers, and hands encoded output to FFmpeg. Renderer events can flow to notification backends. The desktop GUI, CLI, profile store, and job manager use shared configuration/rendering types rather than separate renderer implementations.

The principal code boundaries are `render.rs`/`video_export.rs` and export helpers; `watch.rs`/`watch_runtime.rs`/`runtime.rs`; `sono_gcc.rs`; `config_profiles.rs`; `job_manager.rs`; `render_ui.rs`; `watermark.rs`; `notifications.rs`; `ffmpeg.rs`; `gui.rs`; and `main.rs`. This is a practical module separation, though some orchestration and large UI modules remain substantial.

## 2. Major subsystem integration

**Integration-tested.** The all-target suite includes real Watch execution, CPU/WGPU rendering paths, MP4/audio output, profile and job behavior, cancellation/retry/recovery, UI/watermark composition, and notification routing. A fresh CLI export also exercised Watch, frame rendering, FFmpeg, FFprobe, and SFX audio together. No production-code integration defect was found in this pass.

This was a validation/documentation pass, not a line-by-line formal verification or a full rewrite of every subsystem. Resource and concurrency conclusions below are bounded by the tests and direct runs listed here.

## 3. Configuration architecture

**Unit-tested, integration-tested, and CLI-verified.** `RenderConfig` is the shared serialized configuration used by the desktop application and CLI. Defaults, validation, path resolution, and rendering fields are covered by tests and documented in `docs/USER_CONFIGURATION.md`. The CLI's `default-config`, `validate-config`, and `render-config` flows were exercised; direct help was checked against the newly linked release executable.

Configuration includes renderer inputs/output, timing, dimensions/FPS, CPU or WGPU selection, interpreter or Sono-GCC selection, scheduling/workers, layers, gameplay UI, watermark, and notifications. Existing fields and frontend controls were not removed during this final pass. Historical compatibility for every configuration ever produced was not exhaustively tested.

## 4. Profile integration

**Unit-tested and CLI-verified.** Full-render profiles persist validated `RenderConfig` snapshots. The isolated-store CLI workflow successfully created, listed, showed, exported, imported, selected, deselected, and deleted a profile. Profile resolution validates and normalizes the concrete config before rendering or queue submission. Tests cover profile CRUD/import/export and configuration snapshot behavior.

Profile data uses a versioned format; imported names/configuration are validated and relative resources are resolved from the profile file. No GUI click-through of profile editing was performed in this pass.

## 5. Job queue integration

**Unit-tested, integration-tested, and partially CLI-verified.** Jobs retain an effective config snapshot and profile identity at submission. Tests cover output reservation/collision checks, persistence/recovery, cancellation, retry diagnostics, failure isolation, state transitions, and renderer-backed execution. A fresh CLI workflow created a profile, submitted a real Larp configuration with `--enqueue-only`, listed/inspected it, cancelled and removed it, and cleaned up the isolated store.

The CLI smoke test did not wait for that submitted job to render. The test suite supplies the renderer-backed job execution evidence. The configured default is one queue worker; this pass did not run a long concurrent-job stress or crash/restart soak.

## 6. VM/Sono-GCC integration

**Unit-tested, integration-tested, and directly verified.** The interpreter remains the default semantic path and fallback. Sono-GCC directly implements a bounded scalar subset and routes stateful/effectful or unsupported work through ordered Watch VM cuts. The existing differential tests cover native operation results and mixed VM cuts, including state mutation, lazy branches, audio, spawn, and display-list effects. Cache identity includes graph/runtime/compiler inputs; cold compile, warm cache, and fallback paths have tests.

Six real-engine VM/GCC fixture comparisons and a 20-frame Larp SFX whole-chart parity run are documented in the existing [Sono-GCC performance report](SONO_GCC_PERFORMANCE_REPORT.md) (**existing benchmark**, not rerun here). In this pass, a fresh real Larp 2-second export produced byte-identical VM and GCC MP4 output and matching FFprobe stream properties. GCC remains opt-in; these measurements do not establish an end-to-end speedup.

## 7. Gameplay UI integration

**Integration-tested and visually inspected.** The renderer's gameplay UI path is profile-driven and composited by the real renderer. Existing validation manifests and PNGs include an asset-based special-hit state at 1280×720 and 1024×768, plus UI states with non-default values and event fixtures. The 16:9 and 4:3 outputs were opened and inspected; the key score/life/combo/judgement components are present, relative placement survives both aspect ratios, and there is no obvious major clipping or overlap in these inspected frames. Validation events are synthetic external fixture state, not claims about authoritative Watch results.

Evidence and reproduction metadata are in [`artifacts/ui-validation/MANIFEST.md`](artifacts/ui-validation/MANIFEST.md). Visual inspection was limited to the saved frames listed there; this is not a full visual review of every profile/state/resolution.

## 8. Watermark integration

**Unit-tested, integration-tested, and visually inspected.** Watermark configuration participates in the shared render config and renderer compositing. Tests cover the master switch (including fixed attribution and moving watermark both disabled), deterministic seeded movement, profile assets/fonts, and compositor behavior. The saved moving-watermark output was inspected; its bottom-left bitmap text is pixelated/small but readable at the validation resolution. No repeated full-video determinism comparison was run specifically for watermark in this final pass.

See [`artifacts/watermark-validation/MANIFEST.md`](artifacts/watermark-validation/MANIFEST.md) and the [watermark profile guide](watermark-profiles/README.md).

## 9. Notification integration

**Unit-tested; live delivery not tested.** Renderer and job events route through the notification configuration. Tests use fake backends to exercise filters/templates, native/Discord routing, failure isolation, and configuration behavior. Delivery errors are treated as auxiliary and do not fail the render path in the covered tests. Discord credentials are configured by environment-variable name rather than included in profile exports; no real token or live Discord request was used.

OS notification appearance and asynchronous delivery were not interactively tested. See [`docs/NOTIFICATIONS.md`](docs/NOTIFICATIONS.md).

## 10. CLI integration

**Build-checked and directly verified.** The fresh release binary's top-level and relevant subcommand help was inspected. Profile CRUD/import/export/select flows and queue submit/list/inspect/cancel/remove were run against isolated temporary stores. The `render-video` CLI performed paired real-engine VM/GCC exports and the outputs were checked independently with FFprobe. Temporary outputs and stores were removed after checks.

Not every legacy option was manually exercised one by one; existing CLI parsing and behavior are also covered by the 16 CLI tests in the full suite.

## 11. Main GUI integration

**Build-checked and unit-tested; interactive behavior not tested.** The release build and all-target checks compile the desktop GUI. Headless tests cover applying shared configuration/profile state and relevant background GCC preparation behavior. No interactive desktop click-through was performed, so profile selection, queue operation, settings interaction, and GUI rendering were not visually validated in this pass.

## 12. Performance results

**Measured in this pass.** Direct paired 2-second Larp exports at 640×360, 12 FPS, CPU backend, with SFX/BGM enabled and UI/particles disabled, completed in 3.06 s for interpreter and 4.03 s for Sono-GCC in this sample. This is a single export sample per mode and should not be treated as a stable benchmark; GCC was about 31.7% slower in this end-to-end sample.

An isolated-cache, one-second Watch benchmark ran three interpreter trials and one cold plus two warm GCC trials. Interpreter Watch times were 163, 163, and 167 ms (process times 0.902, 0.893, and 0.913 s). GCC cold Watch was 179 ms and total process time 67.590 s, including 66.143 s compilation. Warm GCC Watch was 138 and 143 ms, with 1.010 and 1.028 s process times. The warm Watch portion improved, while total process time remained slower than interpreter; cold compilation dominates first-use latency. This short benchmark used Larp only and is not a whole-application workload matrix.

Earlier controlled real-fixture and whole-chart results, with their sample sizes and limits, are recorded in the linked GCC report. No fresh GPU throughput benchmark, cold engine-load benchmark, queue-overhead benchmark, or notification-latency benchmark was collected here.

## 13. VM/GCC benchmark results

**Verified for output parity; performance conclusion is opt-in.** The paired direct exports had identical MP4 SHA-256 `8058ea0a8247e96f77d1ebbecf9850bd3c0466d88d97d8c573428a011e3438f3`, 24 H.264 frames at 640×360, 2-second duration, AAC audio, and mixed SFX. Renderer FFprobe validation and an independent FFprobe invocation passed. The exact byte match is specific to this fixture/configuration.

The existing benchmark report records a prior full 20-frame Larp SFX VM/GCC comparison with equal RGB and SFX event hashes and an identical MP4 hash; that is **existing benchmark evidence**, not a new whole-chart run for this final pass. This pass did not produce a fresh full-chart export. Fresh isolated Watch timing does not establish overall GCC speedup.

## 14. Regression results

**Verified.** `cargo test --offline --all-targets` completed with **301 passed, 0 failed, 3 ignored**. This includes 172 library passes, 16 CLI tests, 3 math integration tests, 101 renderer tests, and 9 scheduler tests. The three ignored real-corpus diagnostics were then run serially and all **3 passed**: DISPLAYHOLIC dynamic-stage trace, DISPLAYHOLIC MV alpha audit, and Horizon SDB particle ledger. The guarded runner backed up and restored their existing forensic artifacts. Those diagnostics took 609 seconds in total and are resource-heavy; they are not part of the ordinary fast suite.

`cargo clippy --offline --all-targets` exited successfully with existing warnings, including deprecated API usage, unused helpers/instrumentation, and style/complexity suggestions. Warnings remain technical debt; they were not mass-edited during a stabilization pass.

## 15. Integration-test results

**Integration-tested.** In addition to the full suite and ignored real-corpus diagnostics, this pass directly exercised fresh CLI profile and queue workflows and a real-engine video export. UI and watermark artifacts were visually opened. Existing tests exercise real Watch and renderer/encoding paths rather than relying only on mocks; notification delivery tests appropriately use fakes. No live Discord/OS notification was sent.

## 16. Platform/build results

**Build-checked on Windows only.** `cargo build --release --offline` passed; `cargo check --release --offline --all-targets` passed; `cargo fmt --all --check` and `git diff --check` passed before final report creation and are rerun for the final repository state. The installed target available in this environment is `x86_64-pc-windows-msvc`.

Linux/macOS were not cross-built. Their GUI/native-notification branches are present in source but unverified. Automatic FFmpeg provisioning is Windows-only and the documented non-Windows workflow is limited; do not infer cross-platform support from the Windows build.

## 17. Compatibility results

**Unit-tested and partially integration-tested.** Existing render commands, shared config fields, and execution modes remain represented; legacy interpreter behavior is retained as default/fallback. Serde/default/config tests and CLI regression tests pass. The README was corrected to remove the unsupported “flawless recordings” claim and to document current config, profile, UI, watermark, notification, GCC, and platform behavior. A full historical corpus of user configs and every command-line permutation was not available for exhaustive migration testing.

## 18. Security findings

**Basic source/config review only.** Profile imports validate schema/version, names, and normalized render configuration; resource paths are explicitly resolved, and custom assets can refer to external local paths by design. Discord bot credentials are referenced through environment configuration and were not placed in logs or profile exports in the tested paths. FFmpeg provisioning uses a pinned Windows distribution with validation. No live secrets were used.

This was not a penetration test, fuzzing pass, or exhaustive subprocess/path threat model. Imported profiles that reference local assets still require ordinary user trust in those paths.

## 19. Known limitations

- No fresh complete whole-chart VM/GCC export was performed in this final pass; the fresh export was a 2-second real-engine segment.
- GUI behavior was not click-tested. Live native notifications and Discord were not exercised.
- No Linux/macOS build, sanitizer/Miri run, fuzz pass, or long-duration concurrency/resource soak was available.
- WGPU rendering is covered by tests, but no fresh WGPU performance profile was collected.
- The three ignored diagnostics require private local fixtures and are slow/resource intensive; the MV diagnostic peaked around 1.35 GB working set during this run.
- The direct end-to-end timing comparison is a small sample; it is evidence against claiming GCC is faster overall, not a general performance estimate.
- Existing Clippy warnings remain.

## 20. Remaining technical debt

The repository's largest practical risks are cross-platform validation gaps, cold GCC compilation cost, limited whole-render performance instrumentation, substantial GUI/render-UI module size, and warnings from unused/deprecated instrumentation helpers. The slow private-corpus tests would benefit from clearer opt-in documentation and a controlled lower-resource diagnostic mode, but their assertions were preserved. No code rewrite or speculative optimization was warranted by the evidence collected.

## Final checklist status

| Area | Result |
|---|---|
| Existing renderer workflow and audio/video output | **Verified** on the real Larp segment; full suite also passes |
| VM/GCC semantic and byte output parity | **Verified** for tested fixtures; fresh segment byte-identical |
| GCC fallback/cache | **Unit-tested and integration-tested**; cold/warm cache benchmarked |
| Profiles and immutable job config snapshots | **Unit-tested**; CLI profile workflow verified |
| Queue, cancellation, retry, persistence/recovery | **Unit/integration-tested**; CLI management workflow verified |
| Gameplay UI and watermark | **Integration-tested**; representative PNGs visually inspected |
| Notifications | **Fake-backend tested**; no live delivery |
| CLI | **Build-checked and directly exercised** |
| GUI | **Build-checked and headless-tested**; not interactively tested |
| Full normal suite and ignored private diagnostics | **301 passed, 3 ignored in suite; all 3 ignored then passed separately** |
| Release build | **Passed** |
| Linux/macOS, long soak, sanitizer/fuzz | **Not tested** |

## Reproduction commands

```powershell
cargo fmt --all --check
cargo check --offline --all-targets
cargo test --offline --all-targets
cargo build --release --offline
cargo clippy --offline --all-targets
```

Run private corpus diagnostics only where the referenced local test fixtures exist; they may take several minutes and overwrite saved artifacts unless those artifacts are backed up and restored. The exact real-render and GCC benchmark commands and fixture paths are in `SONO_GCC_PERFORMANCE_REPORT.md` and the manifests under `artifacts/ui-validation/` and `artifacts/watermark-validation/`.
