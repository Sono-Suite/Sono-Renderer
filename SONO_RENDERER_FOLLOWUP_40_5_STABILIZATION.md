# Sono-Renderer Follow-up #40.5 — Stabilization and Regression Triage

**Evidence date:** 2026-10-08  
**Revision:** `fe8dc1e` (`0.1.0`)  
**Scope:** verify the #40 changes and current release state; no product feature work.

## 1. Baseline and #40 handoff

The repository was already a dirty development tree at `fe8dc1e`; it had 133 changed or untracked status entries before this report was added. The package manifest also records the tree as dirty. This is not a clean release baseline. The current product and test state was checked against [the feature audit](FEATURE_EXISTENCE_AUDIT.md), [the #39 certification](RELEASE_CANDIDATE_CERTIFICATION.md), [the #40 closure report](POST_CERTIFICATION_GAP_CLOSURE.md), [known limitations](docs/KNOWN_LIMITATIONS.md), the #37/#38 reports, and the source.

The #40 Presentation Studio close guard is present in `src/gui.rs`. Normal close of a dirty composition is intercepted and offers **Keep Editing**, **Save Project and Close**, **Save Recovery Checkpoint and Close**, and **Discard and Close**. Save/checkpoint/removal errors leave the window open and show an error. Close while jobs are cancelling is deferred until pending work settles. Checkpoint polling occurs from app-level `eframe::App::logic`, so switching away from Presentation Studio does not suspend it. Chart Studio retains its own document-specific Save/Discard/Keep Editing dialog; it does not share a new persistence subsystem with Presentation Studio.

The update in #40.5 is test coverage only: `src/composition.rs` now checks that an invalid canonical save leaves the previous `.sonocomp` bytes intact, a recovery checkpoint is stored separately, and an invalid checkpoint write leaves both the canonical project and previous valid checkpoint unchanged. No production behavior was changed in this pass.

## 2. Test-failure investigation

The #40 handoff recorded an earlier parallel full-test invocation with a failing `--lib` target, but did not preserve the failing test name or assertion in its artifact. Searching the available repository/target logs found no surviving original failure output, so the historical test cannot now be named or root-caused. It is not appropriate to label that failure “flaky” or “fixed.” Status: **NOT REPRODUCED — ROOT CAUSE UNCONFIRMED**.

Current reproduction evidence:

| Run | Result |
|---|---|
| #40 isolated library rerun | 403 passed, 8 ignored, 0 failed (recorded in #40 report) |
| #40 complete serial suite | 543 passed, 8 ignored, 0 failed (recorded in #40 report) |
| #40.5 complete parallel suite, `cargo test --locked --no-fail-fast` | 543 passed, 8 ignored, 0 failed; full output preserved in [`full-parallel.log`](artifacts/rc405-stabilization/full-parallel.log) |
| #40.5 complete serial suite, `cargo test --locked -- --test-threads=1` | 543 passed, 8 ignored, 0 failed; full output preserved in [`full-serial.log`](artifacts/rc405-stabilization/full-serial.log) |
| Focused recovery preservation test | Passed after the final test change: 1 passed, 0 failed |
| Repeated WGPU preview parity test | 2/2 passed, about 33 seconds each |
| Repeated bounded frame-pipeline hash/FIFO test | 2/2 passed, about 64 seconds each |

One test invocation initially collided with the parallel Cargo build because both commands attempted to link the same test executable. Windows `link.exe` reported `LNK1104` for `target/debug/deps/renderer-5c2715eb02e5401f.exe`; no test started in that failed invocation. Cargo work was then run sequentially, and the focused test passed. This is a reproducible shared-target command collision, not a renderer test failure or process abort. No tests were deleted or weakened.

The full current serial run includes the final test-only recovery assertions. No failures or abnormal process exits were present in either captured full-suite log.

## 3. Windows process stability and native boundaries

The #40.5 full parallel suite and the repeated WGPU/pipeline runs produced no observed `STATUS_HEAP_CORRUPTION`, `STATUS_ACCESS_VIOLATION`, abnormal harness termination, or failed test process. The prior isolated/serial clean runs are additional evidence. This does **not** establish that the historical intermittent Windows issue is fixed; its original failure artifact and root cause remain unknown.

Static review found the explicit Windows unsafe/native loader boundary in `src/sono_gcc.rs`: `LoadLibraryW`, `GetProcAddress`, `FreeLibrary`, and conversion of resolved DLL addresses into the compiled-region function pointer. A `NativeLibrary` RAII owner retains the DLL and temporary directory for the lifetime of the compiled program. This path is opt-in Sono-GCC and was not the VM used for the new resource sample. `src/export_control.rs` owns spawned media children, polls cancellation, kills and waits for a child on cancellation/error, and has a test for reaping a cancelled FFmpeg process. The render pipeline similarly owns FFmpeg stdin/stderr and staging files. These inspections found no evidence explaining the historical abort; they are not a formal FFI safety proof or a soak test.

## 4. Memory and resource measurements

Two bounded release-executable renders used the largest checked-in compressed level fixture: [`bike.json.gz`](TestingSuite/Next%20Sekai%20Engine/levels/bike/bike.json.gz), 144,414 bytes. Configuration was: `Next Sekai Engine.zip`, `ProSeka Faithful 0.8.3.scp`, that level, the `larp 64x.mp3` positional input with `--no-bgm`, interpreter VM default, CPU pixel backend, no SFX/particles, start 0 seconds, duration 1 second, 2 fps, 160×90. The output contains two video frames and no audio stream. The unrelated music path was supplied only because the direct-render CLI requires the positional argument; BGM was disabled.

| Run | Wall time | Samples (~150 ms) | Peak working set | Peak private bytes | Other sampled data |
|---|---:|---:|---:|---:|---|
| 1 | 6.9 s | 41 | 252.8 MiB | 317.7 MiB | Approx. 89% of one logical CPU over the sampled interval |
| 2 | 6.9 s | 39 | 260.6 MiB | 319.4 MiB | 6 peak threads, 141 peak handles, host-wide `ffmpeg.exe` process-name count peaked at 1 |

Both files were 10,337 bytes and FFprobe validated H.264, 160×90, 2 fps, 1.000 second. Run 1 SHA-256: `9DE9D1E5B2D460FF25EBFDC22044308647A4EB2F112FE935C8A5B9B1FCB9E2`. The sample CSVs, logs, and output are in [`artifacts/rc405-large-chart/`](artifacts/rc405-large-chart/). The release process exited after each render; a follow-up process check found no remaining process named `renderer` or `ffmpeg`.

I extracted and visually inspected [`bike-first-frame.png`](artifacts/rc405-large-chart/bike-first-frame.png), SHA-256 `3D32775A9B21E7BE0946CBC57101210CDB0DC22AF97A7DC81C48E0BEAA59F855`. At 160×90, the gameplay is visible but extremely small/dark; HUD and text detail cannot be judged. No frame-boundary clipping is apparent at this diagnostic scale. This is a render/resource smoke inspection, not visual-quality approval.

This is bounded evidence, not a leak test. The two peak private-memory samples are close, but two short renders cannot establish long-run boundedness or explain the previously observed multi-gigabyte Task Manager reduction. GPU memory, process I/O bytes, temp-staging peak usage, cancellation cleanup under load, and multi-minute chart preview were not measured. WMI access was denied, preventing process-tree/physical-device attribution; the FFmpeg count is host-wide sampling, not a definitive child-process trace. No memory leak is claimed or ruled out.

The serial library test process was separately sampled during the latter portion of its test run; sampling began after 364 of 411 test cases had completed. Across 349.5 seconds and 1,325 samples, the test runner reached 1,004 MiB working set and 990.4 MiB private bytes, with 16 threads and 354 handles at peak. It was about 476 MiB working set at the end of the sample, immediately before the test process exited. The sample averaged about 97% of one logical CPU; the harness was serial, but runtime tests can use internal worker threads. See [`serial-lib-resource.csv`](artifacts/rc405-test-resource/serial-lib-resource.csv). This identifies a high temporary test-workload footprint; it does not establish the full-run peak or prove a leak. It also does not reproduce the reported 12–13 GiB machine-wide change.

## 5. #40 data-safety verification

| Behavior | Evidence and status |
|---|---|
| Save before close | Source-inspected: the canonical project save must clear dirty state before close is issued. UI interaction was not automated. |
| Discard | Source-inspected: removes a stale recovery checkpoint first; on removal error the window remains open. No GUI click test. |
| Keep editing | Source-inspected: clears the close prompt and keeps the document dirty/open. Decision predicate is unit-tested. |
| Save a recovery checkpoint | Atomic versioned checkpoint path and separate canonical `.sonocomp` are tested; successful UI action marks the close as checkpointed. |
| Invalid canonical save | New assertion: invalid project save returns an error and preserves canonical bytes; focused recovery test passes. Filesystem/ACL failure after staging was not fault-injected. |
| Invalid recovery write | New assertion: invalid checkpoint write leaves existing valid checkpoint bytes intact and leaves canonical bytes unchanged; focused test passes. |
| Close during a pending operation | Source-inspected: close is deferred while owned jobs/capture settle and close cancellation is coordinated. Native UI timing was not exercised. |
| Abnormal-exit recovery | Snapshot load/schema/corruption tests pass; no GUI process-kill/relaunch drill was available. |
| Canonical project preservation | Test saves an actual `.sonocomp`, writes/updates a separate recovery file, and compares canonical bytes after normal and failed writes. |
| Dirty-state reset | Source-inspected: later edits clear the one-shot checkpoint-close marker. No interactive stale-state exercise. |
| Chart Studio | Existing independent document close flow offers save, discard, keep editing. The close UI itself was not manually exercised. |

The existing recovery snapshot serialization test was strengthened; the focused test passed after the final source change. Close-prompt logic and all project tests pass in the full test runs. The 20-second idle-checkpoint policy still leaves edits since the last checkpoint vulnerable to an abrupt crash/power loss; only a normal close receives the new explicit resolution prompt.

## 6. Build and regression results

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Pass after final test edits |
| `cargo check --locked --all-targets` | Pass after the final test-only assertion |
| `cargo clippy --locked --all-targets` | Exit 0; existing warnings remain (library tests reported 193 warnings, 126 duplicates) |
| `cargo build --locked --release --bin renderer` | Pass, optimized build completed; `rc.exe` absent so Windows version metadata was not embedded |
| Full parallel suite | 543 passed, 8 ignored, 0 failed: lib 403/8 ignored, main 27, `arctan2` 3, `m0` 101, scheduler 9, doctests 0 |
| Full serial suite | 543 passed, 8 ignored, 0 failed: library 403/8 ignored (765.99 s), main 27, `arctan2` 3, `m0` 101 (100.62 s), scheduler 9 (2.58 s), doctests 0 |
| Ignored tests | 8 library tests remain ignored; 3 private-corpus tests were identified in #39 as unavailable. No ignored-test count was increased in this pass. |

The five real-FFmpeg integrations were rerun in #40.5 against the current tree and all passed:

| Integration | Result | Captured output |
|---|---:|---|
| `composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export` | 1 passed, 70.59 s | [`composition real-media log`](artifacts/rc405-stabilization/composition-tests-real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export.log) |
| `composition::tests::real_ffmpeg_template_reuse_across_ordinary_sources_portrait_and_gameplay` | 1 passed, 26.11 s on the preservation-safe rerun (also passed in 25.85 s) | [`template stdout`](artifacts/rc405-stabilization/template-test.stdout.log), [`template stderr`](artifacts/rc405-stabilization/template-test.stderr.log) |
| `subtitles::tests::real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm` | 1 passed, 0.37 s | [`subtitle extraction log`](artifacts/rc405-stabilization/subtitles-tests-real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm.log) |
| `timeline_playback::tests::video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes` | 1 passed, 0.51 s | [`video-pipe log`](artifacts/rc405-stabilization/timeline_playback-tests-video_pipe_keeps_animation_and_text_filter_files_alive_until_decode_finishes.log) |
| `project::tests::saved_project_can_queue_and_render_a_real_fixture` | 1 passed, 7.76 s | [`queued project-render log`](artifacts/rc405-stabilization/project-tests-saved_project_can_queue_and_render_a_real_fixture.log) |

The template integration writes to a fixed relative artifact path. To avoid overwriting the pre-existing `artifacts/presentation-template-validation` directory, the final rerun used an isolated working directory under `target/rc405-template-isolated-rerun`; the test's `TestingSuite` fixture path was linked to the repository fixture tree. Its generated PNG/MP4/project/template outputs remain in that ignored `target` directory. The other four tests write to their normal temporary locations. These tests cross production compositor/export code, but do not substitute for a manually operated GUI session.

## 7. Existing workflow acceptance (A–G)

| Workflow | #40.5 evidence | Assessment |
|---|---|---|
| A. Original renderer | Fresh Larp 64x direct render through release executable, CPU backend and interpreter produced H.264/AAC, 320×180, 6 fps, 12 frames, 2.000 seconds; SHA-256 `A47A176E90D8BE44B8E804BFBD3A7BC29F6047B5265C8A599828437C949DA785`, matching #39. Metadata validated and a frame from this fixture was previously inspected. The new `bike` render was a bounded stress sample with BGM off, not another complete musical acceptance. | **Real-engine/render path passed for named fixture; generic Watch support remains partial.** `media time = Watch time + bgmOffset` remains covered by conversion tests; this render alone does not prove all SFX/offset behavior. |
| B. Ordinary NLE | #40.5 reran the production FFmpeg ordinary video/audio project preview and MP4 export test (70.59 s), plus the saved-project-to-queue real fixture render test (7.76 s). No Sonolus inputs are used by the standalone ordinary media case. | **Production path integration-tested; GUI interaction unavailable.** |
| C. Chart Studio | Deterministic genuine LevelData gzip export/reload and runtime fixture tests are present and passed in the full suite. No manually authored chart was driven through GUI close/reimport/Watch/render. | **Core format/runtime boundaries tested; claimed spatial authoring workflow remains partial/unverified.** |
| D. Presentation templates | #40.5 reran template reuse across ordinary sources, portrait output, and pre-rendered gameplay video. Visual spot checks covered the landscape preview, portrait preview, gameplay portrait preview, and transition midpoint. The large central gameplay inset is visibly small, and the portrait ordinary-media layout leaves substantial dark/blurred space; these are limitations of the sample/template presentation, not evidence that the template path is missing. | **Template/composition path integration-tested and visually inspected; no live Watch integration. Layout quality is not certified.** |
| E. Subtitles | #40.5 reran real FFmpeg source-audio extraction with trim/rate/mono conversion (0.37 s); SRT/WebVTT and timed-text persistence/edit tests passed in full suites. | **Editing/extraction boundaries tested; no model was installed, so no speech transcription claim.** |
| F. BPM | Synthetic tempo, silence, phase, and apply/undo behavior tests passed. | **Algorithmic test evidence only; no labeled real-song review or interactive chart round trip.** |
| G. Queue/history/manifest | Saved project → real queue render passes; job, history, and manifest persistence/serialization tests pass. | **Subsystem boundaries tested; one end-to-end history → manifest → close/reopen → reproduce GUI flow not exercised.** |

## 8. Preview/export parity

The #40.5 real-media NLE tests passed, including template variants, ordinary media, a pre-rendered gameplay-video source, portrait output, and standalone audio/video export. I opened the generated landscape preview, ordinary portrait preview, gameplay portrait preview, and landscape transition midpoint PNGs. Inspectable copies are saved in [`artifacts/rc405-stabilization/visual/`](artifacts/rc405-stabilization/visual/): [landscape](artifacts/rc405-stabilization/visual/ordinary-landscape-preview.png), [portrait](artifacts/rc405-stabilization/visual/ordinary-portrait-preview.png), [gameplay portrait](artifacts/rc405-stabilization/visual/larp-gameplay-portrait-preview.png), and [transition midpoint](artifacts/rc405-stabilization/visual/ordinary-landscape-transition-midpoint.png).

| Visual artifact | SHA-256 |
|---|---|
| `ordinary-landscape-preview.png` | `FD975E8AD5218C675D49707396553FE724B56B417588D8A183DE1945526EFB7F` |
| `ordinary-portrait-preview.png` | `0B8253DDE5E9B1A49E665E5896905EA886F33852ED90323FA1353F65EE7E931D` |
| `larp-gameplay-portrait-preview.png` | `FDF4A47040999837C9B5A064E8147C713C3791F0CEA055481EA91CCDA924F13B` |
| `ordinary-landscape-transition-midpoint.png` | `2609CCE77AA6AD5142A84F8AB2FF4BAD39F03D83E0CA35542BA86959595CF225` |

The renders show the intended blurred background, inset/source views, cyan graphic, and caption; the transition midpoint shows both source views during the dissolve. I did not see frame-edge clipping in these samples. The gameplay inset is notably small in portrait, the ordinary portrait composition is mostly dark/blurred negative space, and the tiny central details are not visually legible at the displayed resolution. These outputs are genuine rendered evidence, not a visual-quality pass; the presentation remains sparse and is not certified as polished. The full automated suite also passed current Watch/render and deterministic pipeline checks. No new preview/export defect was found in this stabilization pass. This is not certification of every cross-product listed in #37/#40: trim + speed + transition, nested relink + proxy/original export, mask + transform + effect, animated styled captions, and simultaneous gameplay/media overlays were not all manually compared at representative timestamps. Lossy output is validated semantically/through the existing tests, not claimed pixel-identical.

## 9. Package, licensing, and distribution

The current Windows x64 development portable package is [`Sono-Renderer-0.1.0-windows-x86_64-portable.zip`](target/rc40-final-review-fe8dc1e/distribution/Sono-Renderer-0.1.0-windows-x86_64-portable.zip), 54,476,544 bytes, SHA-256 `DAEF6AE6D8D78F9EEB3F793D7DE376CC47B993E9E902858CFA534F2736346216`. Its manifest says `dirty: true`, `reproducible_build: false`, commit `fe8dc1ed910c85edc27e72bedd242ed3c06ca186`. The package smoke test passed Unicode/space extraction paths, foreign working directory, restricted PATH, and packaged `--version`, help, jobs, and media-tool-info CLI flows. This was the developer host, not a clean VM. It did not launch the GUI or render media using package-discovered FFmpeg.

FFmpeg/FFprobe are not bundled; this environment used the shared local add-on pair. Fresh/offline first-export discovery is not certified. `ISCC.exe` and `rc.exe` are absent; no installer, PE version resource, signed package, install/upgrade/uninstall, or clean-machine GUI was tested. The sandbox denied the renderer's per-user performance-history/diagnostic directory under `AppData\Local`, and native notification delivery failed during the direct render. The MP4 still completed, but those persistence/notification paths are not validated by that run.

The package includes generated third-party notices for 196 Cargo crates, but 16 license text files were missing from the local registry archives. The repository MIT license does not establish rights for bundled artwork/media; the `/assets` pack provenance/redistribution terms remain undocumented. FFmpeg distribution/download obligations and third-party fonts/media also require review. No missing permission or license term was guessed. Legal/distribution review remains a hard external blocker.

## 10. Updated open register

Statuses use the required classifications. “Fixed and verified” is scoped to the automated/source evidence described above; it does not imply unperformed GUI/device testing.

| ID | Current issue | #40.5 status |
|---|---|---|
| G01 | Presentation Studio dirty-close loss path | **FIXED AND VERIFIED** in decision/persistence tests; native close UI remains unexercised |
| G02 | Abnormal GUI crash/relaunch | **BLOCKED BY ENVIRONMENT** — no interactive GUI kill/relaunch harness |
| G03 | Incomplete generic Watch operations | **INTENTIONAL LIMITATION** pending authoritative semantics and differential fixtures; not safe to guess |
| G04 | Engine-specific spatial Chart Studio workflow | **INTENTIONAL LIMITATION**; generic LevelData boundaries pass, full GUI authoring not certified |
| G05 | Live Watch-backed NLE source | **INTENTIONAL LIMITATION**; pre-rendered gameplay remains ordinary media |
| G06 | Advanced NLE customization gaps | **INTENTIONAL LIMITATION**; not expanded in stabilization |
| G07 | Portrait template visual density | **INTENTIONAL LIMITATION**; valid output but prior visual audit calls it sparse |
| G08 | Manual Presentation Studio workflow | **BLOCKED BY ENVIRONMENT** — no interactive desktop automation/session |
| G09 | Windows installer and install lifecycle | **BLOCKED BY ENVIRONMENT** — Inno Setup `ISCC.exe` unavailable |
| G10 | Embedded PE version resource | **BLOCKED BY ENVIRONMENT** — Windows SDK `rc.exe` unavailable |
| G11 | Clean-machine packaged GUI | **BLOCKED BY ENVIRONMENT** — no clean Windows VM/host |
| G12 | Fresh/offline FFmpeg use | **BLOCKED BY ENVIRONMENT / EXTERNAL REQUIREMENT** — local shared pair available; bootstrap/offline behavior and redistribution not certified |
| G13 | Asset-pack redistribution rights | **BLOCKED BY EXTERNAL REQUIREMENT** — rights-holder/provenance evidence needed |
| G14 | Sono-GCC performance claim | **INTENTIONAL LIMITATION** — experimental; #39 cold/warm Larp data showed no speedup |
| G15 | Physical microphone | **BLOCKED BY ENVIRONMENT** — no real input device/permission/hot-plug test |
| G16 | Real speech/model validation | **BLOCKED BY ENVIRONMENT** — no approved local model/sample corpus installed |
| G17 | yt-dlp network acquisition | **BLOCKED BY ENVIRONMENT** — executable/network fixture unavailable |
| G18 | Labeled real-song BPM quality | **BLOCKED BY EXTERNAL REQUIREMENT** — licensed ground-truth corpus/reviewer required |
| G19 | Physical GPU/package validation | **BLOCKED BY ENVIRONMENT** — no identified hardware adapter/clean package host |
| G20 | Stable release source identity | **BLOCKED BY EXTERNAL REQUIREMENT** — maintainer must review/commit clean source, add matching release notes, and create version tag |
| G21 | Dependency legal notice completeness | **BLOCKED BY EXTERNAL REQUIREMENT** — 16 upstream license texts/expressions require review |
| G22 | Close during job/audio | **FIXED AND VERIFIED** at source/decision level; native asynchronous close interactions untested |
| G23 | Crash bundle/safe mode/file association | **INTENTIONAL LIMITATION**; not required to close a demonstrated #40 defect |
| G24 | Full network → chart → mic → transcription → MP4 chain | **BLOCKED BY ENVIRONMENT** — missing network, device, model, interactive GUI, and complete generic chart authoring |
| G25 | Clippy warnings | **NOT REPRODUCED** as a build/lint failure; Clippy exits 0, but existing warnings remain |
| G26 | Linux/macOS release packages | **INTENTIONAL LIMITATION** — the current supported release target is Windows x86_64 |

## 11. Remaining risks and release owners

- **Release maintainer:** review and commit the intended dirty tree, add `release-notes/v0.1.0.md`, and tag the exact source. Until then the produced ZIP is a development artifact, not an RC candidate.
- **Windows release engineering:** provision `rc.exe` and Inno Setup, build/version the installer, configure and verify signing as required, then test install/upgrade/uninstall and the packaged GUI.
- **Legal/release owner:** document rights for supplied assets/fonts/media, retrieve and verify the 16 missing dependency license texts, and review FFmpeg/runtime-download obligations before publication.
- **QA/release owner:** run a clean Windows VM installed-GUI path for first launch, ordinary video/audio edit/export, direct Sonolus render, and recovery. Current smoke is CLI-only on the developer machine.
- **Renderer/compatibility owner:** retain accurate scope for unsupported Watch operations, generic chart authoring limits, and incomplete advanced NLE controls. Any expansion needs authoritative semantic inputs and targeted differential tests.
- **Platform/equipment owners:** real microphone, speech-model, yt-dlp/network, and physical-GPU validation remains required only for claims made about those optional workflows; absence here is not evidence of an implementation defect.
- **Reliability owner:** preserve the historical Windows parallel-test event as unresolved. Current complete parallel/serial and repeated targeted runs pass, but no root cause or permanent fix was identified. Do not close the incident as fixed.

Known remaining correctness risk: a crash/power loss can lose edits since the last idle checkpoint. Failed-write validation is covered, but disk-full/ACL/atomic-replace fault injection and native GUI recovery after an actual process kill were not available. The constrained direct render also could not persist performance/diagnostic history or deliver notifications.

## 12. Final re-certification status

`cargo fmt --all -- --check`, all-target check, Clippy, release build, full parallel and serial tests, repeated focused WGPU/pipeline tests, recovery preservation test, five rerun real-media FFmpeg integrations, direct Larp render, bounded large-chart render, and current portable package CLI smoke have positive evidence as described above. The real-media outputs were visually inspected with the specific readability/layout reservations documented in sections 4, 7, and 8. Eight ignored unit tests remain ignored; three private-corpus cases were unavailable. The full serial run's final outcome is recorded below.

### Final serial all-target result

`cargo test --locked -- --test-threads=1` completed successfully: 543 passed, 8 ignored, 0 failed. The 411-case library harness had 403 passes and 8 ignored in 765.99 seconds; the 27 main tests, 3 `arctan2` tests, 101 `m0` tests, 9 scheduler tests, and zero doctests all passed. Full output: [`artifacts/rc405-stabilization/full-serial.log`](artifacts/rc405-stabilization/full-serial.log).

## 13. Release recommendation

The stabilization run did not find a reproducible product regression or Windows process abort. It also did not resolve the unknown historical abort, establish long-run memory behavior, or certify interactive/clean-machine workflows. The existing product limitations and external release blockers remain material; a green test suite alone does not make this dirty development artifact releasable.

**NOT READY FOR RELEASE CANDIDATE**
