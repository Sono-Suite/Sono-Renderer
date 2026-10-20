# Post-#48 Release Closure

**Execution date:** 2026-10-08  
**Overall state:** release closure is incomplete. This is an evidence ledger, not a release approval.  
**Source state:** the checkout was already broadly dirty before this pass. No reset, cleanup, commit, or tag was performed. The development package manifest records commit fe8dc1ed910c85edc27e72bedd242ed3c06ca186 and dirty: true.

## Executive result

The post-#48 suite is stable in this run, and the available real-media integration tests plus the Windows portable package and packaged admin-log API smoke passed. The concrete #47 gap where the admin Logs view showed only job-owned records was closed by connecting bounded process log capture to the existing API and React page.

The repository is still not a releasable candidate. The Watch compatibility audit contains unresolved semantics; #37 customization and #43 localization are still partial; asset and third-party redistribution rights need human review; the release is built from a dirty checkout; rc.exe and Inno Setup are unavailable; and there was no clean-machine installer or interactive GUI certification. A green Cargo suite does not close those gates.

## Source inspection and baseline

The register was derived from the actual #48 convergence ledger, #39 certification, #40.5 stabilization, #36 feature audit, #37/#47 reports, current code, package scripts, known-limitations documentation, and current Windows tool availability. Older reports were treated as leads and checked against present paths; their desired-state descriptions were not treated as proof.

The architecture remains the existing renderer, Watch/VM, Sono-GCC, Presentation Studio, projects/jobs, admin API, and web client. This pass did not introduce a parallel job, render, timeline, or configuration system. No Sonolus execution or render semantics were changed. The only production behavior added here is process-level admin log capture and presentation of those records in the existing Logs page.

Before production edits, both all-target baselines passed: serial and parallel each reported 576 passed, 8 ignored, 0 failed. The checkout was already dirty. The baseline parallel run did not reproduce the historical Windows test-process abort.

## Complete gap register

Status terms are deliberately evidence-based: **FIXED AND VERIFIED**, **NOT REPRODUCED**, **PARTIAL**, **STILL MISSING**, **BLOCKED BY ENVIRONMENT**, **BLOCKED BY EXTERNAL REQUIREMENT**, and **INTENTIONAL LIMITATION**.

| ID | Gap and source | Classification | Work/evidence in this pass | Final status and remaining closure |
|---|---|---|---|---|
| C01 | Intermittent Windows parallel test abort from #39/#40.5/#48 | Reliability / test | Baseline and final serial and parallel all-target runs completed. No heap-corruption, access-violation, or harness-abort signature occurred. | **NOT REPRODUCED.** No root cause was found; a clean run is not a fix. Keep tracking on additional Windows hosts and runs. |
| C02 | Project, autosave, checkpoint, cancellation, and recovery data safety | Persistence / correctness / test | Full suite passed, including existing persistence and recovery tests. No forced GUI crash, process termination during save, disk-full simulation, or power-loss test was performed. | **PARTIAL.** No defect reproduced; crash/filesystem-fault recovery remains **BLOCKED BY ENVIRONMENT** and requires explicit fault-injection evidence. |
| C03 | Sonolus v1.1.4 Watch semantics: stack operations, replay host provisioning, StreamSet edges, curved Draw rasterization, and some particle/presentation cases | Semantic correctness / compatibility | Follow-up #48 added bounded Replay Data decode/injection and all six curved Draw operations. The pragmatic Watch closure then implemented the approximate stack, random, and mutable-stream behavior in the existing WatchVm, with strict/permissive policy, job warnings, analyzer status, and GCC exclusion tests. | **PARTIALLY CLOSED.** Local implementation and focused tests now cover the formerly missing operation families. Official-client parity remains unverified; particle, lifecycle, timing, and render-mode evidence remains incomplete. |
| C04 | Broad Sono-GCC compatibility and speed claims | Semantic correctness / performance / test | The full suite passed; prior #45 evidence supports narrow parity only and preserves VM as oracle and fallback. This pass made no compiler changes and ran no new comparative Performance Lab study. | **PARTIAL / EXPERIMENTAL.** Keep GCC opt-in and make no general speed claim. Broader engine coverage and separate compile/cache/load/execute measurements remain. |
| C05 | #37 Presentation Studio customization completeness | Implementation / integration / UX | Full suite and real-media composition/export fixtures passed. No new editor primitives were added in this release-closure pass. | **STILL MISSING.** The #37 report and current capability matrix still list ordinary customization gaps. Do not describe the editor as having the full requested composable customization set. |
| C06 | Reusable template, responsive layout, and user editing workflow | Integration / UX / test | The real-media template test passed across unrelated ordinary sources and portrait/landscape cases. The test covers production composition/export code, not a mouse-driven packaged GUI workflow. | **IMPLEMENTED BUT NOT FULLY VALIDATED.** Media-path evidence is positive; interactive editability and packaged GUI use remain untested. |
| C07 | Application localization coverage from #43 | Implementation / test / documentation | Packaged locale resources validate: 10 release locales, 2 developer locales, 73 canonical message keys. Existing validator also ran from the package smoke. | **STILL MISSING.** Catalog validity is not full application localization; many GUI, error, accessibility, admin, and CLI strings remain English. Human translation review is also outstanding. |
| C08 | Admin Logs exposed job history but not process events (#47/#48) | Integration / UX / security | Added an optional bounded log-facade collector; wired process events and retained job events into the existing protected API; updated the Logs UI with source/severity/target/job identity, capture status, and five-second refresh. Unit/API tests, packaged root/JS/CSS/health routes, and a packaged loopback log API smoke passed. | **FIXED AND VERIFIED for process-local capture.** The process ring holds 2,000 records in memory and clears on exit. Durable cross-restart process logs are not implemented; durable render/job history remains with its existing owner. |

### Watch semantics Continuation B reassessment (2026-10-08)

The current Watch closure evidence was rechecked against the official function references and the checked-in oracle packet. The public contracts now establish `Random`'s inclusive range, `RandomInteger`'s half-open range, the `StreamSet` argument/return shape, and a 0–4095 Temporary Memory range. They still do not define the stack pointer/frame ABI, random seed/state lifecycle, or mutable-stream collision and callback semantics. Corrected valid-arity oracle fixtures exist, but this environment has no Sonolus client executable available and there is no complete valid-client transcript. No production behavior was guessed or changed. The Watch gap remains **STILL BLOCKED / NOT RELEASE-READY**; see `WATCH_SEMANTICS_CLOSURE.md` for the operation-by-operation evidence and required probes.
| C09 | Admin authorization, redaction, and remote operation | Security / test / environment | Admin tests passed in the full suite. Packaged smoke verified loopback log reads, active capture, the startup event, and HTTP 400 for an invalid severity filter. | **PARTIAL.** Loopback only was exercised. Remote HTTPS/token authentication, certificate rotation, browser behavior, and network exposure were not tested; do not expose it publicly on this evidence. |
| C10 | Release identity, installer, and clean-machine packaging (#39/#48) | Packaging / external requirement / environment | Built a development-channel Windows x64 portable ZIP and manifest, then smoke-tested the extracted archive under a Unicode/space path, foreign working directory, and restricted PATH. The packaged CLI and admin API ran. | **BLOCKED BY ENVIRONMENT / EXTERNAL REQUIREMENT.** The artifact says dirty: true; version is 0.1.0; no matching tag or release-notes/v0.1.0.md; rc.exe and Inno Setup ISCC.exe are unavailable; no installer or clean Windows VM test exists. |
| C11 | Third-party and packaged-asset redistribution provenance | Legal / external requirement | Generated a package notice for 255 Cargo registry packages. It explicitly flags 16 registry archives without license text and states that asset rights are not resolved by the Cargo notice. | **BLOCKED BY EXTERNAL REQUIREMENT.** Maintainers/rightsholders must verify the 16 texts and separately packaged artwork/media/font rights before public distribution. No legal conclusion is made here. |
| C12 | FFmpeg/runtime discovery and offline first-run packaging | Packaging / test / external requirement | Real-media tests passed using FFmpeg discoverable in this development environment. The portable smoke deliberately restricted PATH and validated CLI operation, not video export. The package does not bundle FFmpeg. | **PARTIAL.** A user may need the documented managed FFmpeg acquisition path on first MP4 export. Clean offline export and redistribution policy were not certified. |
| C13 | Physical microphone, real speech/model, network acquisition, BPM corpus, physical GPU | Environment-only validation / test | FFmpeg subtitle audio extraction passed. No physical microphone, actual local Whisper model/speech corpus, yt-dlp network acquisition, labeled BPM corpus, or GPU-specific capability matrix was exercised in this pass. | **BLOCKED BY ENVIRONMENT.** These items are not called implementation failures solely because hardware/network/model data was unavailable. Their real-device and model acceptance evidence is still required. |
| C14 | Interactive GUI, project reopen after crash, and clean installed workflow | UX / reliability / packaging / environment | Cargo GUI/model tests and packaged CLI/API smoke passed. No interactive desktop automation, visual inspection, clean-user profile, installer, or GUI crash/relaunch exercise was available. | **BLOCKED BY ENVIRONMENT.** A developer desktop/manual and clean-machine run remain a release gate for claimed GUI workflows. |
| C15 | Documentation and product-claim consistency | Documentation | Updated admin-console and known-limitations docs to describe process log capture and its in-memory lifetime. Package documentation and locale validation were exercised. | **PARTIAL.** The central known-limitations and feature reports still accurately enumerate unresolved Watch, NLE, localization, packaging, and validation work; no broad product claims were reworded as complete. A maintainer must reconcile all public release notes after fixing those gaps. |

## Production changes

### Admin process log integration

- src/app_log.rs owns a process-local ring bounded to 2,000 entries. Target and message sizes are bounded on UTF-8 boundaries. Severity, target, timestamp, and sequence are retained. Values of nonempty environment variables whose names identify common credential fields are redacted.
- Capture is installed only when the optional admin service starts. A pre-existing logger from an embedding host is not replaced; the API reports that capture is inactive in that case. Job lifecycle records remain owned by the job manager and are not migrated into this ring.
- src/admin.rs merges process records with the existing job event contract in /api/v1/logs; it supports text/severity/limit filters and rejects unknown severity values. The API continues to use its existing authorization boundary.
- src/main.rs routes top-level and notification diagnostics through the standard log facade while preserving existing stderr output. GUI progress events are forwarded through the same facade when capture is enabled.
- web/src/main.tsx adds process/job source, severity, target and job identity display, a capture-availability notice, and bounded five-second refresh while the Logs page is open.
- docs/ADMIN_CONSOLE.md and docs/KNOWN_LIMITATIONS.md state that process events are memory-only and cleared at exit.

This is not a durable log archive. Redaction covers values from recognizable secret environment-variable names; it cannot prove that arbitrary application messages never contain a secret that came from another source. Do not log credentials or sensitive media paths. Remote security was not newly certified.

## Final validation evidence

| Validation | Result |
|---|---|
| cargo fmt --all -- --check | **Passed.** Cargo emits an environment warning that it cannot canonicalize C:\Users\Admin; formatting exit code was 0. |
| cargo check --locked --all-targets | **Passed** after production edits. |
| cargo clippy --locked --all-targets | **Passed with 199 warnings and no errors.** The warning set was not established as wholly pre-existing. |
| Final serial cargo test --locked --all-targets -- --test-threads=1 | **Passed after Watch closure production edits:** library 449, CLI 27, arctan2 3, m0 103, Watch scheduler 9; **591 passed, 8 ignored, 0 failed**. |
| Final parallel cargo test --locked --all-targets | **Passed:** same **591 passed, 8 ignored, 0 failed**. No STATUS_HEAP_CORRUPTION, access violation, or harness-abort marker. This clean run does not establish the historical Windows abort is fixed. |
| Real FFmpeg subtitle extraction, trim/speed/mono PCM | **Passed:** 1 test within real-ffmpeg-tests.log. This tests preparation, not recognition accuracy. |
| Real-media standalone NLE composition and MP4 export | **Passed:** 1 real FFmpeg test. |
| Real-media template reuse across ordinary sources and portrait composition | **Passed:** 1 real FFmpeg test. This is media-path evidence, not GUI interaction or full gameplay Watch-source evidence. |
| Persistent project to queued real fixture render | **Passed:** 1 ignored integration test run explicitly. |
| FFmpeg video filter-file lifetime test | **Passed:** 1 ignored regression test run explicitly. |
| TypeScript type-check and Vite production build | **Passed** before packaging; Vite required its supported runner config loader under this restricted environment. Vite output was generated under web/dist and included in the package. |
| Locale validation | **Passed:** 10 release locales, 2 developer locales, 73 canonical keys. Does not certify coverage or human translation quality. |
| Release-profile Windows build | **Passed** through the package script. rc.exe was unavailable, so Windows executable version metadata was not embedded. |
| Development portable package | **Built:** Windows x64 portable ZIP, 56,824,214 bytes; manifest SHA-256 e90a8ab4af5d4560502293554cd529b0a1a88b7f5771c8c6b817a49aa6d31c14. Manifest truthfully records dirty source and non-reproducible build. No installer was produced because ISCC was absent. |
| Portable package smoke | **Passed:** extraction, Unicode/space path, foreign CWD, restricted PATH, version/help/jobs/media-tool-info, packaged assets/docs/locales. This is not a clean-machine GUI or offline MP4 export test. Log: artifacts/release-closure/final-20261008/package-smoke.log. |
| Packaged admin web/API smoke | **Passed:** packaged admin run served HTML (427 bytes), JavaScript (253,878 bytes), CSS (12,143 bytes), health status, and the process-log API on loopback. API reported active capture and 2,000 capacity, returned the process startup event, and rejected invalid severity with HTTP 400. The smoke process was stopped after requests; remote TLS was not tested. |
| Third-party notice inventory | **Generated** for 255 registry packages. Sixteen lacked license text in their registry archives; packaged artwork/media rights remain unverified. |
| Interactive GUI, clean installed application, clean-machine/offline MP4, physical devices, remote TLS | **Not tested / unavailable** in this environment. |
| Performance Lab before/after comparison | **Not run.** This Watch closure added curved GPU UV remapping; no comparable Performance Lab baseline was collected, so no performance conclusion is claimed. |

Full test summaries include real-engine cases such as the materially different engine Chart Studio roundtrip and Watch preview and the Next RUSH Baumkuchen/Armageddon shared-preview fixtures. These do not close the separate incomplete v1.1.4 operation audit or constitute a fresh packaged end-to-end render for every supported engine.

## Updated release matrix

| Area | Previous #48 position | Evidence now | Updated position |
|---|---|---|---|
| Serial/parallel regression suite | Historical intermittent abort; not reproduced in prior pass | Both final suites pass, 580 passed / 8 ignored each | **NOT REPRODUCED; continue monitoring** |
| Project/data safety | Implementation tests existed, GUI/fault evidence limited | Full suite passes; no destructive fault injection or GUI crash/recovery test | **PARTIAL / NOT FULLY VALIDATED** |
| Watch v1.1.4 | Incomplete semantics in audit | Replay decoder/injection and curved Draw have local tests; stack/random/StreamSet now execute through WatchVm approximations with warning/strict policy. No client oracle was available. | **IMPLEMENTED WITH LIMITATIONS; EXACT PARITY UNVERIFIED** |
| Sono-GCC | Narrow parity, experimental breadth | No code changed; suite passes; no new broad parity/performance study | **EXPERIMENTAL / PARTIAL** |
| Presentation Studio customization | #37 matrix contains partial/absent capabilities | Real composition/export fixtures pass; no additional primitives or interactive certification | **PARTIAL** |
| Templates and responsive outputs | Implemented; interactive workflow uncertain | Real-media template/portrait path passes | **IMPLEMENTED; GUI VALIDATION OPEN** |
| Localization | Partial | Catalog syntax/packaged validation passes, 73 keys only | **PARTIAL** |
| Admin process logs | Job-only view identified as a gap | Unit/API tests, frontend build, packaged loopback API pass | **IMPLEMENTED, PROCESS-LOCAL** |
| Admin remote operations | Security code exists; no remote certificate client evidence | Loopback only | **PLATFORM/SECURITY VALIDATION OPEN** |
| Windows release package | No clean release artifact/installer validation | Dirty development ZIP built and portable smoke passed; no ISCC or rc.exe | **DEVELOPMENT PACKAGE ONLY; NOT RELEASE-READY** |
| FFmpeg | Works in development, clean/offline uncertain | Real-media tests pass using locally discoverable runtime; package smoke used restricted PATH but no export | **PARTIAL / OFFLINE PACKAGE VALIDATION OPEN** |
| Legal provenance | Unresolved | 255 Cargo notices generated; 16 texts missing; assets unreviewed | **BLOCKED BY EXTERNAL REVIEW** |
| Mic/STT/network/BPM/GPU | Real hardware/model/network evidence unavailable | No relevant real-device corpus/hardware tests in this pass | **BLOCKED BY ENVIRONMENT** |
| GUI/installer/clean machine | Not validated | Packaged CLI and admin API only | **BLOCKED BY ENVIRONMENT** |

## Exact continuation work

The next release-closure run should proceed in this order:

1. **Establish release identity:** preserve and review the current dirty changes, produce a deliberate reviewed commit, add matching 0.1.0 release notes/tag only after approval, and rebuild from the clean tagged tree. Do not reuse this dirty development artifact as a stable release.
2. **Resolve redistribution rights:** obtain/verify the missing 16 upstream license texts and written rights/provenance for every packaged asset/font/media item. Keep the generated notice as an inventory, not as a legal approval.
3. **Provide Windows release tools:** install the supported Windows SDK resource compiler and Inno Setup, produce the installer, validate version metadata, install/uninstall from a clean Windows VM/user profile, and rerun package smoke from a path with spaces and non-ASCII characters.
4. **Close remaining Watch parity evidence:** compare the implemented stack, Random/RandomInteger, and StreamSet approximations against authoritative versioned v1.1.4 client traces when available; also compare replay attachment, curved Draw pixels, and visual/particle behavior. Keep approximate operations in the single WatchVm and preserve VM fallback; do not promote them to exact parity without evidence.
5. **Close established UI/product claims:** complete the still-partial #37 NLE workflows and #43 user-facing localization through current production architecture; update the matrices only after interactive save/reopen/preview/export checks.
6. **Run real acceptance sessions:** exercise clean/offline FFmpeg first export, packaged GUI startup and direct project open, mic capture with a physical input, speech recognition with the pinned model and real speech/silence/music cases, yt-dlp with permitted network sources, labeled BPM fixtures, GPU capability, and remote admin over TLS with an authenticated client.
7. **Re-run the full release checklist:** serial and parallel suites on a clean Windows host, release build/package, installer smoke, real engine/VM/GCC parity, NLE export/preview parity, GUI workflows, and any changed-area Performance Lab measurements. Preserve raw logs and artifact hashes.

## Limitations and decision

This pass does not claim the full #39/#48 certification was repeated: only the available relevant subset was rerun. It does not claim an installer was built, an interactive GUI was exercised, the native parallel abort was fixed, Watch semantics are complete, localization is complete, Sono-GCC is broadly proven faster, every optional device/model/network path works, or legal redistribution is cleared.

The positive evidence supports continuing toward a release candidate, not issuing one. The unresolved Watch correctness items, incomplete established NLE/localization claims, dirty source identity, absent installer toolchain/clean-machine proof, and unresolved licensing are sufficient blockers.

NOT READY FOR RELEASE CANDIDATE

## Follow-up: Watch semantics closure

The focused Watch follow-up is recorded in `WATCH_SEMANTICS_CLOSURE.md`. It preserves the single authoritative `WatchVm` and did not change Sono-GCC native eligibility.

- Replay Data now has a bounded gzip/JSON decoder and can be attached before execution through `WatchRuntime`, `WatchFrameStepper`, and `FrameSession`. It remains optional; ordinary renders do not require replay data.
- The six `DrawCurved*` operations now dispatch through the existing VM and generate ordered tessellated patches. CPU and GPU paths sample the corresponding source UV regions. Ordinary `SpriteDraw` serialization/hash behavior is retained for the default full-texture case.
- Focused regression tests cover replay decoding and query behavior, malformed/untrusted replay values, late attachment, curved VM dispatch/geometry, legacy serialization, and CPU/GPU UV cropping.
- Validation after production edits: formatting, all-target check, Clippy, serial and parallel all-target suites, release build, and explicitly selected ignored FFmpeg/project integration tests passed. The all-target runs each reported **591 passed, 8 ignored, 0 failed**; Clippy reported 199 warnings and no errors.
- No valid target-client Replay Data trace or curved-draw pixel reference was available. Fourteen stack operations, `Random`/`RandomInteger`, and `StreamSet` remain unsupported. Exact client parity for curved rasterization, replay integration, lifecycle, numeric edges, and several visual/audio behaviors remains open.
- No Performance Lab comparison was collected, and the historical Windows test-process abort remains unresolved despite clean serial and parallel runs.

This narrows, but does not close, the Watch compatibility blocker. Overall release status remains **NOT READY FOR RELEASE CANDIDATE** for the reasons above and the separate packaging, provenance, GUI, and product-validation blockers recorded in this report.

## Follow-up: pragmatic Watch compatibility (2026-10-08)

The current implementation closes the local execution gap for stack
operations, `Random`, `RandomInteger`, and `StreamSet` without claiming exact
official-client parity. They are classified `Approximate`, run only through
the authoritative WatchVm, and emit one warning per family per render job.
The permissive policy is the backward-compatible default; strict policy
rejects these operations before evaluating their arguments. Render diagnostics
retain the warnings. CLI users can select the policy and deterministic random
seed on `run-watch` and `render-video`; the advanced render settings expose
the strict/permissive policy in the desktop GUI.

Focused tests passed for all stack operation arms and memory bounds, random
range/repeatability, stream ordered upsert and query behavior, strict-mode
argument side-effect suppression, warning deduplication, diagnostics
association, old RenderConfig deserialization, analyzer partial status,
CLI policy/seed flags, and Sono-GCC exclusion. The compatibility report schema
is now version 2 to identify the new `approximate` per-operation status while
retaining its legacy top-level fields. The actual Sonolus client executable
and a complete v1.1.4 oracle transcript were not available, so sequence,
pointer/frame ABI, and callback/replay lifecycle parity remain unverified.
This changes the implementation status, not the release recommendation: the
independent packaging, provenance, GUI, and device/network validation blockers
in this report remain open.

### Validation performed for this closure

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed; Cargo warned that it could not canonicalize `C:\Users\Admin`. |
| `cargo check --locked --all-targets` | Passed; the build script warned `rc.exe` is unavailable, so Windows executable version metadata was not embedded. |
| `cargo clippy --locked --all-targets` | Passed with the repository's existing warning set and no errors. |
| Focused Watch VM operations | Passed: 5 tests. |
| Warning deduplication into job log | Passed: 1 runtime integration test and 1 ExportControl test. |
| RenderConfig old-file defaults and roundtrip | Passed: 1 test. |
| CLI strict/permissive policy and seed parsing | Passed: 1 test. |
| Analyzer approximate status / dynamic probe | Passed in the full serial run before it was interrupted. |
| Full serial all-target suite | **Incomplete.** It passed the real three-engine Chart Studio roundtrip/render fixture and many following tests, then was interrupted after the ARMAGEDDON CPU/WGPU comparison remained active for over five minutes without emitting a result. No assertion failure or abort was observed before interruption. |
| Release build | Passed: `cargo build --locked --release`. |
| Exact Sonolus client differential | Unavailable: no target client executable or complete valid oracle transcript was present. |
