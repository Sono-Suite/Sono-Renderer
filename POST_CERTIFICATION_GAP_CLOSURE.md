# Sono-Renderer Post-Certification Gap Closure

## Initial gap register (before changes in this pass)

This register was extracted from [the #39 certification](RELEASE_CANDIDATE_CERTIFICATION.md), [the #36 audit](FEATURE_EXISTENCE_AUDIT.md), [the #37 report](SONO_RENDERER_FOLLOWUP_37_REPORT.md), [the #38 report](SONO_RENDERER_FOLLOWUP_38_REPORT.md), [the Sonolus capability audit](docs/SONOLUS_V1_1_4_CAPABILITY_AUDIT.md), and [known limitations](docs/KNOWN_LIMITATIONS.md). Status and feasibility below describe the repository/environment before this pass's implementation. It separates missing behavior from missing evidence.

| ID | Feature / initial status | Expected behavior / why #39 did not certify | Gap class and severity | Feasibility, dependency, resolution |
|---|---|---|---|---|
| G01 | Presentation Studio unsaved close / **data-loss risk** | Closing after edits but before a 20-second idle checkpoint bypasses both explicit save and recovery prompt. #39 documented this and did not run GUI crash/close recovery. | **CORRECTNESS + DATA SAFETY**, Critical | **Feasible now**, no external dependency. Add a close guard with explicit Save, durable recovery checkpoint, Discard, and Keep Editing choices; test checkpoint preservation and close-decision logic. |
| G02 | Recovery after abnormal GUI termination / **not fully validated** | Recovery snapshots are atomic and versioned, but no forced GUI crash/restart drill was run. | **TEST GAP**, High | Source-level persistence tests are feasible; a real process-kill GUI test depends on interactive GUI automation. Keep canonical project untouched and validate recovery format paths. |
| G03 | Generic Sonolus Watch v1.1.4 operations / **partial** | Stack operations, normal-host replay-data provisioning, and curved Draw rasterization are incomplete. #36 states checked-in references are insufficient to infer semantics safely. | **IMPLEMENTATION + CORRECTNESS**, High | Not safe to guess. Requires authoritative operation/geometry semantics and differential fixtures/oracle captures. Preserve one Watch traversal; collect authoritative evidence before implementing. |
| G04 | Chart Studio authoring scope / **partial** | Generic LevelData entity authoring/export exists; an engine-specific spatial/archetype editor and interactive author→export→reimport→Watch→render GUI workflow were not certified. | **INTEGRATION + UX + TEST**, High | Partially feasible. Existing generic edit/export/runtime test passes; full spatial tools depend on engine-specific schemas/semantics. Validate and document the current generic workflow; do not fabricate archetype meaning. |
| G05 | Live Watch-backed NLE source / **absent by design** | NLE uses rendered gameplay video as ordinary media; it does not host a live Watch session or bind authoritative gameplay state. | **GENUINELY OUT-OF-SCOPE** for the established standalone-NLE contract; medium | High cross-system coupling and a new source lifecycle. Existing product supports gameplay through exported video. Keep deferred unless the product explicitly requires live gameplay composition; never duplicate Watch traversal. |
| G06 | Advanced NLE customization / **partial** | Freeform/compound masks, richer blends/transitions, group inheritance, broader text/style tooling, and some #37 items are absent. Production FFmpeg tests cover the implemented subset, not all requested effects. | **IMPLEMENTATION + TEST**, Medium | Some individual primitives may be feasible, but #39 supplied no failing parity case establishing a release blocker. Avoid a broad unbounded #37 reimplementation in a certification-gap pass. Promote only features with a concrete user-facing claim and focused preview/export evidence. |
| G07 | Portrait template visual density / **valid but sparse** | Generated portrait templates export and fit the canvas, but inspected examples have a small centered inset and large blurred background. | **UX**, Low/Medium | Feasible as a preset/layout revision, but no evidence it is a broken default. Preserve user-authored templates and treat new visual design as product work, not certification repair. |
| G08 | Standalone NLE GUI interaction / **not validated** | Production FFmpeg tests exercise project, preview, and export boundaries, but no human-operated desktop edit session was completed. | **TEST + UX**, High | Requires interactive GUI environment/automation. Not equivalent to source changes; run if available, otherwise retain explicit limitation. |
| G09 | Packaged Windows installer / **blocked** | `ISCC.exe` is absent, so no installer, upgrade, reinstall, or uninstall behavior was exercised. | **PACKAGING + ENVIRONMENT-ONLY VALIDATION**, High | Requires Inno Setup. The portable package can be smoke-tested locally; installer behavior cannot be certified without the compiler. |
| G10 | PE version metadata / **blocked** | `rc.exe` is absent; release executable has no Windows version resource. | **PACKAGING + ENVIRONMENT-ONLY VALIDATION**, Medium | Requires Windows SDK resource compiler or a deliberate build-tool change. Do not make false packaging claims. |
| G11 | Clean-machine GUI / **not validated** | Portable CLI smoke ran on developer host; no clean VM/installed GUI session was available. Rust/Cargo absence was checked only through restricted PATH CLI smoke. | **TEST + ENVIRONMENT-ONLY VALIDATION**, High | Requires clean Windows VM/user. Existing smoke test does not prove GUI, first-run media, FFmpeg acquisition, or render. |
| G12 | FFmpeg on fresh/offline installation / **platform-limited** | FFmpeg/FFprobe are not bundled; #39 found a shared local pair, so clean download/offline first export was not tested. | **PACKAGING + INTEGRATION + TEST**, High | Requires distribution policy, network/cache and licensing review. Verify existing discovery/bootstrap; do not bundle an unreviewed binary or imply offline first export. |
| G13 | Asset/dependency redistribution rights / **unresolved** | Asset pack rights are undocumented; 16 dependency license text entries were missing locally. | **DOCUMENTATION + RELEASE COMPLIANCE**, Critical | Requires rights-holder and dependency-license review, not renderer code. Keep publication blocked until documented; do not infer permission. |
| G14 | Sono-GCC speed / **experimental, not a correctness defect** | VM/GCC outputs matched for one fixture; cold compile (71.296 s) dominated and warm run was slightly slower than VM. | **PERFORMANCE + TEST**, Medium | Measurement says no general acceleration. Keep optional and correctly report compile/cache/load/execute time; no optimization without evidence. |
| G15 | Real microphone capture / **not device-validated** | Device enumeration, permission, hot-plug, latency, drift, and packaged capture require hardware not present. | **ENVIRONMENT-ONLY VALIDATION**, High | No code rewrite justified by #39. Needs physical input device/OS permission test. Existing fake/error-path tests are not real-device evidence. |
| G16 | Local speech transcription / **not model-validated** | Model was not installed; real speech, languages, accuracy, package discovery, and inference were not tested. | **ENVIRONMENT + TEST**, High | Requires an approved local model/runtime artifact and sample speech corpus. No cloud service or surprise model download. |
| G17 | yt-dlp URL acquisition / **not network-validated** | Pinned executable and network were unavailable. Local media works independently. | **ENVIRONMENT-ONLY VALIDATION**, Medium | Requires pinned tool and permitted test URL/network. Do not rewrite acquisition code absent a failure. |
| G18 | BPM analysis quality / **synthetic-only** | Synthetic MP3/silence/phase tests pass; no labeled real-song corpus/interactively reviewed chart workflow. | **TEST GAP**, Medium | Needs licensed/labeled audio corpus and human ground truth. Do not overclaim quality or change detector based only on no-reference tests. |
| G19 | Physical GPU path / **not device-validated** | WGPU tests passed where configured, but physical adapter identity and packaged use were not established. | **ENVIRONMENT-ONLY VALIDATION**, Medium | Requires supported GPU/driver and clean packaged host. CPU/software tests do not substitute. |
| G20 | Windows stable release identity / **blocked** | Dirty checkout, no matching `v0.1.0` tag, and no `release-notes/v0.1.0.md`. | **PACKAGING + PROCESS**, Critical | Requires maintainer source review/commit/tag and approved release notes. An agent cannot truthfully certify the current dirty tree as a clean tagged release. |
| G21 | Dependency notices / **incomplete** | Third-party notices contain 196 crate entries with 16 license-text files missing. | **DOCUMENTATION + RELEASE COMPLIANCE**, Critical | Requires dependency/license audit and source texts or recorded exceptions. Do not manufacture license terms. |
| G22 | GUI close during jobs/audio / **partially guarded** | Current GUI cancels/waits for jobs and finalizes an active mic take, but Presentation Studio unsaved document state was omitted from that close flow. | **INTEGRATION + CORRECTNESS**, High | Feasible with G01; maintain existing cancellation behavior, then show document-specific safe close choices. |
| G23 | Crash bundle / safe mode / file association / **absent** | #39 notes these are not present; no failure in core render/project paths demonstrated that makes them necessary for current release claim. | **GENUINELY OUT-OF-SCOPE** for this gap closure | No implementation without a specific supported workflow requirement. Existing recovery checkpoint remains the relevant project safety mechanism. |
| G24 | Full network→chart→voice→caption→MP4 chain / **not completed** | Requires unavailable network, genuine engine-specific authoring, microphone, speech model, and interactive packaged application. | **INTEGRATION + ENVIRONMENT-ONLY VALIDATION**, High | Can certify component boundaries separately; cannot honestly synthesize missing physical/model evidence. Do not fake a fully green result. |
| G25 | Formatting / lint warnings / **format pass, warnings remain** | `cargo fmt --check` passes after formatting; Clippy succeeds with substantial warnings. | **DOCUMENTATION / MAINTAINABILITY**, Low | Warnings can be cleaned selectively, but broad lint churn has risk and is not a user-visible release blocker absent denied lints. |
| G26 | Linux/macOS packages / **not claimed or configured** | The documented release workflow currently targets Windows x86_64 only; #39 did not test other operating systems. | **INTENTIONAL PLATFORM LIMITATION**, Medium | A cross-platform port is outside the existing release contract and task. Keep platform claims Windows-only; no OS port is required to certify this target. |

### Initial prioritization

1. Fix G01/G22 first because the GUI can close with recent composition edits neither saved nor checkpointed.
2. Preserve the package-output safety fix from #39 and keep clean-release, installer, rights, and missing-environment blockers explicit.
3. Do not implement G03 by guessing VM opcodes or rasterization; obtain authoritative Sonolus semantics first.
4. Treat G04/G06 as bounded product-scope follow-up work only where a specific broken workflow is established. Do not create parallel systems or inflate the matrix with unverified primitives.
5. Keep G05 and G23 deferred as separate product features; they are not required for direct rendering or standalone ordinary-media editing.

## Closure work and evidence

### Implementation / integration changes

- **G01/G22 — composition close safety:** recovery checkpoint polling moved from the Presentation Studio panel into the app-wide `eframe::App::logic` pass. Leaving the editor or showing Welcome no longer suspends a pending checkpoint. Checkpointing still waits during active capture/export.
- A dirty Presentation Studio document now intercepts application close. The user can keep editing, save the canonical `.sonocomp`, save a separate recovery checkpoint and close, or explicitly discard and close. Save/checkpoint errors leave the application open and visible. If a render task is already cancelling, the close decision is deferred until the task settles.
- A checkpoint chosen specifically for close is tracked separately from canonical save state so the subsequent close request does not reopen the same prompt. A later edit clears that one-shot condition.
- **G01 tests:** added a unit test for the close-prompt decision matrix. The existing versioned/atomic recovery test was rerun and passed. The GUI was not interactively automated, so button-level native close behavior remains unverified.
- **G11 smoke-test usability:** `scripts/smoke-test-windows-package.ps1` now falls back to the OS temp directory if CI's `RUNNER_TEMP` is unset. It still resolves and bounds cleanup to that temp root. The first local attempt exposed this issue; after the change the packaged portable smoke passed.
- The #39 package-output preservation change remains in place: outputs require a fresh `-OutputRoot`, and an existing output is refused rather than recursively deleted.
- Updated [Presentation Studio recovery documentation](docs/PRESENTATION_STUDIO.md) and [known limitations](docs/KNOWN_LIMITATIONS.md) to state the new close/checkpoint behavior and the remaining loss window after an abrupt crash between checkpoints.

No Watch, compiler, renderer, audio engine, timeline, project schema, or media format implementation was changed. No ABI or Sonolus semantics were inferred.

### Updated gap matrix

| ID | #39 status | Work in #40 | Validation | Updated status |
|---|---|---|---|---|
| G01 | Data-loss risk | App-wide checkpointing and explicit dirty-close choices | Close-decision unit test; atomic recovery test; full serial suite; native GUI close not exercised | **Implementation gap closed; GUI integration partially validated** |
| G02 | Crash/recovery not fully validated | Same recovery correction; canonical project remains separate | Atomic snapshot/schema test passed; no forced process-kill GUI test | **Partial / test gap remains** |
| G03 | Watch v1.1.4 partial | No guessed opcode or draw geometry behavior | Full serial Watch suite and Larp render pass; no authoritative oracle for missing operations | **Partial; blocked on authoritative semantics** |
| G04 | Chart Studio partial | No archetype semantics fabricated | Existing full suite and real project queue/render pass; no interactive generic chart authoring session | **Partial; authoring UX validation remains** |
| G05 | No live Watch NLE source | No change | Pre-rendered gameplay is still the supported NLE source boundary | **Deferred / outside the current NLE contract** |
| G06 | Advanced NLE customization partial | No broad new effects added without a failing case | Five real FFmpeg tests passed; no new full customization parity matrix | **Partial; advanced items remain** |
| G07 | Portrait layout sparse | No preset redesign | Existing landscape/portrait/gameplay template test passed; output remains visually sparse | **UX limitation remains** |
| G08 | NLE GUI untested | No interactive UI environment available | Production FFmpeg integrations rerun; GUI actions not simulated | **Environment-only validation gap** |
| G09 | Installer unavailable | No Inno compiler available | Development portable only | **Blocked** |
| G10 | PE version resource missing | No `rc.exe` available | Release build warns version resource omitted | **Environment/toolchain blocked** |
| G11 | Clean-machine GUI absent | Smoke runner made usable without CI-only temp variable | Portable CLI smoke passed on developer host with restricted PATH/foreign CWD; no clean VM or GUI | **Partially closed (smoke harness); certification gap remains** |
| G12 | FFmpeg fresh/offline path untested | No unreviewed binary bundled | App's FFmpeg-backed tests and render pass using shared installed pair; no first-run download/offline install | **Platform-limited / unvalidated** |
| G13 | Asset rights unresolved | No legal terms guessed | No change; rights review still required | **Release blocker** |
| G14 | GCC performance uncertain | No speculative optimization | Same Larp output hash; #39 cold/warm measurements still show no speedup | **Experimental, performance claim limited** |
| G15 | Physical mic unavailable | No rewrite | Unit tests pass; no real device | **Environment-only validation gap** |
| G16 | STT model unavailable | No model download/network or cloud path added | Extraction test passes; no real inference | **Environment-only validation gap** |
| G17 | yt-dlp/network unavailable | No source-specific workaround | Local media and packaged tool-info smoke pass; no acquisition | **Environment-only validation gap** |
| G18 | BPM synthetic-only | No detector change without labeled reference corpus | Full synthetic tests pass; no real-song validation | **Test gap remains** |
| G19 | Physical GPU unavailable | No GPU path rewrite | WGPU automated parity tests pass; no hardware/package check | **Environment-only validation gap** |
| G20 | Dirty untagged source / no release notes | No commit/tag/release authorization available | Development manifest reports `dirty: true`, `reproducible_build: false` | **Release blocker** |
| G21 | License notices incomplete | No license text fabricated | Package reports 196 crate entries; missing license-text/asset-rights review remains | **Release blocker** |
| G22 | Close flow omitted Presentation Studio dirty state | Fixed with G01 | Unit test + serial full suite; no GUI interaction | **Implementation gap closed; UI evidence partial** |
| G23 | Crash bundle/safe mode/file association absent | No change | No current core workflow failure requires these features | **Deferred / out of this gap closure** |
| G24 | Full factory workflow not completed | Component verification repeated | Renderer, NLE, persistence, and packaging CLI tested separately; mic/model/network/GUI full chain unavailable | **Not certified end to end** |
| G25 | Clippy warnings remain | No mass warning churn | Clippy exits 0 with warnings | **Maintainability debt remains** |
| G26 | Non-Windows targets not packaged | No port attempted | Windows x64 checks only | **Intentional platform limitation** |

## Re-run of #39 certification

| Check | Result after changes |
|---|---|
| Formatting | `cargo fmt --all -- --check` passed |
| Compile | Focused and all-target `cargo check --locked --all-targets` passed; optimized release renderer rebuilt successfully |
| Clippy | `cargo clippy --locked --all-targets` exited 0 with existing warnings |
| Full test suite | Serial run: 543 passed, 0 failed, 8 ignored (library 403/8 ignored; main 27; arctan2 3; `m0` 101; scheduler 9; docs 0) |
| Parallel test behavior | One initial parallel full invocation reported a failed `--lib` target, but its test name/details were lost in output truncation. An isolated library rerun passed 403/8 ignored, and the subsequent complete serial suite passed. Parallel instability remains unresolved, not declared fixed. |
| Atomic recovery / new close decision | Both focused tests passed; the complete serial suite includes the new decision test |
| Five real-media FFmpeg integrations | All passed again: standalone video/audio export (72.18 s), ordinary/portrait/gameplay template reuse (26.70 s), subtitle source extraction (0.40 s), streaming preview file lifetime (0.55 s), saved project queue/render (7.47 s) |
| Direct renderer | Fresh optimized executable rendered supplied Larp 64x through interpreter/CPU: H.264/AAC, 320×180, 6 fps, 12 frames, 2.000 s; SHA-256 `A47A176E90D8BE44B8E804BFBD3A7BC29F6047B5265C8A599828437C949DA785`, same as #39 baseline |
| Package | Fresh development portable ZIP built; 54,476,544 bytes; SHA-256 `DAEF6AE6D8D78F9EEB3F793D7DE376CC47B993E9E902858CFA534F2736346216`; manifest remains dirty/non-reproducible |
| Portable package smoke | Passed with Unicode/space extraction path, foreign working directory, restricted PATH, version/help/jobs/tool-info. Smoke was run on the developer host, not a clean VM; it did not launch the GUI or render from the package. |
| Installer / release metadata | Not available: no ISCC/installer, `rc.exe` missing, no signing check |
| Interactive/device/network/model | Not available: no GUI session, physical microphone, local speech model, yt-dlp binary/network, or identified physical GPU |

The final fresh portable package was produced at `target/rc40-final-review-fe8dc1e/distribution/Sono-Renderer-0.1.0-windows-x86_64-portable.zip`. An earlier isolated `rc40-review` package was also preserved. No old package output was overwritten.

The final direct-render output is `artifacts/release-candidate-certification-40/classic-larp-vm-cpu-2s.mp4`. Its render succeeded and its codec, dimensions, rate, frame count, duration, size, and hash match the #39 evidence. In this restricted environment, writing per-user performance-history/diagnostic data under `AppData\Local` was denied and native notification delivery failed; these ancillary warnings did not fail video output. Per-user persistence and notification delivery therefore were not validated by this fresh CLI render.

## Remaining implementation and evidence limits

- Generic Watch stack operations, replay stream host provisioning, and curved Draw rasterization remain **partial**. The capability audit says the repository lacks authoritative semantics for safe implementation. They need reference behavior/fixtures before code changes; guessing would threaten VM correctness and violate the single-Watch invariant.
- Chart Studio remains a generic LevelData entity editor, not an engine-specific spatial authoring product. Real export/reimport/runtime integration exists in automated tests, but authoring interaction and engine-specific placement tools are not certified.
- Presentation Studio does not consume a live Watch session. Pre-rendered gameplay MP4s are ordinary media sources. Advanced #37 customizability still exceeds the implemented subset; this pass did not add a second compositor or claim those controls exist.
- The network-to-chart-to-voice-to-caption workflow cannot be honestly validated with the missing network, speech model, hardware, and GUI environment. Component tests do not substitute for it.
- Release blockers from G09/G10/G13/G20/G21 remain. The development portable CLI smoke does not certify a stable tagged installer or clean installed GUI.

## Final release recommendation

The concrete Presentation Studio unsaved-close loss path has been addressed, and recovery no longer depends on the editor being the active tab. The rerun is green in serial, all five available real-FFmpeg integrations pass, the known Larp VM render hash is unchanged, and an updated portable CLI package smoke passes. The implementation is safer than #39's tested snapshot, but the source identity, rights review, installer/toolchain, clean-machine GUI, several partial product features, and real-device/model/network validations remain unresolved. These are not documentation-only statuses and cannot be waived by this code pass.

**NOT READY FOR RELEASE CANDIDATE**
