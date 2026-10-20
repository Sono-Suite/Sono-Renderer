# Sono-Renderer Release Candidate Certification

**Decision: NOT READY FOR RELEASE CANDIDATE**  
**Assessment date:** 2026-10-08  
**Assessed source:** `fe8dc1ed910c85edc27e72bedd242ed3c06ca186`, version `0.1.0`, Windows x64/MSVC

This is a certification of the checkout and validation environment that were actually available. It is not a claim that every requested feature is complete. The checkout is dirty, has no `v0.1.0` release tag or matching release notes, and could produce only a development portable package. A successful renderer build and real-fixture run do not remove those release blockers.

## Evidence labels

- **Tested:** automated test or command completed successfully in this environment.
- **Real-media tested:** exercised with actual project/media/engine inputs, not only a mock.
- **Visually inspected:** generated image/frame was opened and inspected; this does not certify polish.
- **Build-checked:** compiled or packaged, without claiming runtime behavior beyond the stated smoke test.
- **Unavailable:** required hardware, model, tool, account, or clean environment was not available.
- **Not tested:** no adequate evidence was produced during this certification.
- **Partial/experimental:** some paths work, while product or platform coverage is insufficient for a stable broad claim.

## 1. Baseline and environment

The repository inspection used the current feature audit, Follow-ups #37 and #38 reports, architecture, installation, release-process, Presentation Studio, and audio-input documentation. The state is substantially broader than a classic renderer, but multiple product areas remain partial; see [FEATURE_EXISTENCE_AUDIT.md](../follow-ups/FEATURE_EXISTENCE_AUDIT.md), [Follow-up #37](../follow-ups/SONO_RENDERER_FOLLOWUP_37_REPORT.md), [Follow-up #38](../follow-ups/SONO_RENDERER_FOLLOWUP_38_REPORT.md), and [known limitations](../../KNOWN_LIMITATIONS.md).

| Item | Observation |
|---|---|
| Source identity | Commit `fe8dc1e`; version `0.1.0`; no `v0.1.0` tag |
| Worktree | Dirty: 131 changed/untracked status entries at certification; not eligible for a reproducible stable release |
| Platform | Windows 10 x64, MSVC target; this is the only configured release target |
| Rust | `rustc`/`cargo` 1.99.0; Windows target installed |
| FFmpeg | Shared local FFmpeg/FFprobe pair found by the app, version banner `N-126644-g88f7fd9ec5-20260918`; not on PATH and not bundled in this development package |
| yt-dlp | Pinned tool unavailable; no URL acquisition was tested |
| Windows resource compiler | `rc.exe` unavailable; release binary lacks embedded Windows version-resource metadata |
| Installer compiler | Inno Setup `ISCC.exe` unavailable |
| GPU/device identity | Physical GPU identity unavailable; WGPU software/CPU and test-controlled paths are not a substitute for hardware certification |
| CPU identity | WMI query denied; exact CPU model unavailable |
| Release notes | `release-notes/v0.1.0.md` absent; only the template exists |

The tree was dirty before this certification. `cargo fmt --all` was run to satisfy the release workflow after the initial formatting check found formatting differences, particularly in `src/gui.rs`. The resulting formatter check passes; this was formatting-only, not an intentional behavior change. Clippy completes successfully but emits warnings, so warning cleanup is not certified.

## 2. Build and automated validation

| Check | Result | Evidence and limit |
|---|---|---|
| `cargo fmt --all -- --check` | **Pass** | Run after applying repository formatting |
| `cargo check --locked --all-targets` | **Pass** | Windows x64 development/check build |
| `cargo clippy --locked --all-targets` | **Pass with warnings** | No lint errors; library tests report 193 warnings (126 duplicates), binary tests 13 warnings |
| `cargo test --locked --no-fail-fast` | **Pass** | 542 passed, 0 failed, 8 ignored across targets; 3 ignored private-corpus tests were unavailable |
| `cargo build --locked --release --bin renderer` | **Pass, release caveat** | Optimized renderer built; warning that `rc.exe` is unavailable, so Windows version resource was omitted |
| Five ignored FFmpeg real-media tests | **Pass** | Run explicitly after build: standalone NLE export, template reuse/responsive outputs, subtitle audio extraction, decode-pipe lifetime, and saved-project queue/render |

The full test totals were: library 402 passed/8 ignored, main 27, arctan2 3, Watch `m0` 101, scheduler 9, doctests 0. The three remaining ignored tests require private DISPLAYHOLIC/Horizon material. No test failure or abnormal test-harness abort occurred in this run. This clean run does **not** establish that the historical Windows parallel-test instability is fixed.

The five explicit FFmpeg test results were:

- Standalone video/audio project, preview and MP4 export: pass (75.38 s).
- Template reused with ordinary sources, portrait output and gameplay MP4: pass (28.07 s).
- Subtitle source extraction with trim, speed and mono PCM: pass (0.38 s).
- Decode pipe retains animation/text filter files until completion: pass (0.55 s).
- Saved project queues and renders a real fixture: pass (7.50 s).

## 3. Representative renderer and backend evidence

The real `larp 64x` fixture rendered through the optimized Windows executable using the CPU renderer, interpreter VM, and locally discovered FFmpeg. The generated MP4 is H.264/AAC, 320×180, 6 fps, 12 frames, 2.000 seconds, 61,322 bytes. Its SHA-256 is `A47A176E90D8BE44B8E804BFBD3A7BC29F6047B5265C8A599828437C949DA785`, matching the earlier checked-in audit result. A representative frame was extracted and visually inspected; gameplay and attribution are visible. At 320×180 it is a small diagnostic render, not a visual-quality certification. See [the frame](../../../artifacts/release-candidate-certification/classic-larp-midpoint.png).

The Sono-GCC cold and warm runs produced the exact same MP4 hash as the VM run for this fixture. Cold GCC compiled 25,834 regions / 43,404 eligible nodes, executed 248,746 native scalar regions, and took 73.972 s total, including 71.296 s compilation; VM took 2.106 s. With the persistent cache warm, GCC took 2.356 s versus 2.106 s for VM. This is evidence of parity on one fixture, not general backend equivalence or a speedup. GCC remains optional/experimental; do not market it as faster based on this measurement.

Warm-cache GCC run evidence is in `artifacts/release-candidate-certification/classic-larp-gcc-warm-2s.log`; cold-run evidence is in `classic-larp-gcc-2s.log`. The classic VM, cold GCC, and warm GCC outputs share the hash above.

## 4. Feature and workflow release matrix

| Product area | Certification result | Release assessment |
|---|---|---|
| Classic direct render (engine + level + resources/music → MP4) | **Real-media tested** on Larp CPU/VM; two-second output validated with FFprobe and hash-matched to prior audit | Ready only for this narrow known fixture/path; not a full compatibility claim |
| Watch VM / generic Sonolus v1.1.4 | **Tested**: one authoritative Watch path and broad unit/regression suite passed | **Partial/experimental**. Audit documents missing stack operations, replay-stream provisioning, and curved Draw rasterization; do not claim full v1.1.4 compatibility |
| Sono-GCC | **Real-media tested** cold/warm on Larp, byte-identical MP4 to VM | **Experimental**. One workload; cold path dramatically slower and warm path slightly slower than VM |
| Chart Studio / LevelData | **Tested**: real exported level was re-imported through authoritative Watch runtime | **Partial**. Generic entity authoring is not an engine-specific spatial chart editor; full GUI author-edit-preview-export was not interactively exercised |
| Standalone Presentation Studio | **Real-media tested**: ordinary video/audio project preview/export through FFmpeg and persistent project queue render | **Partial/experimental** as a user-facing release claim; desktop GUI was not interactively exercised |
| Reusable templates / responsive outputs | **Real-media tested and visually inspected** on ordinary landscape, ordinary portrait, and gameplay portrait MP4s | Valid outputs, but portrait layout appears sparse: small central gameplay over a large blurred field. Not polished/interactive visual certification |
| Presentation Studio effects/masks/transitions/customization | **Partial** per current audit. Some ordinary NLE and template paths pass | No broad certification of advanced mask/effect/transition parity or all customizability claims |
| BPM analysis | **Tested** with synthetic MP3/silence/phase cases | No labeled real-song corpus or interactive chart-workflow certification |
| Automatic subtitles | **Tested** for timed-text/SRT/VTT and FFmpeg source extraction, including Unicode coverage in unit tests | Speech model was not installed; real-speech transcription, language quality, and packaged model workflow unavailable |
| Microphone recording | **Build/unit tested** for device error paths, staged WAV/finalization/recovery integration | No physical microphone, permission, hot-plug, latency, drift, or real-device test |
| URL acquisition | Local-media paths tested | yt-dlp unavailable; network/source acquisition unavailable |
| Projects, workspace/profile/history persistence | **Tested** through serialization/migration and saved-project queue/render integration | Automated data paths passed; GUI repair/recovery and prolonged crash tests not performed |
| Onboarding, command/workspace navigation | **Tested** by automated logic/command tests | No interactive keyboard/mouse, high-DPI, screen-reader, or installed first-run verification |
| WGPU/GPU | Automated WGPU parity/tests passed where configured | Physical adapter identity and packaged GPU validation unavailable |
| Windows portable package | **Build-checked** and restricted-PATH CLI smoke tested | Development-only package; not a clean machine or GUI certification |
| Windows installer/update/uninstall | **Unavailable** (`ISCC.exe` absent) | Blocked |

The fresh template exports are 1920×1080 and 1080×1920 H.264 at 12 fps for two seconds. Their respective hashes are `E1B3AD7316781C761E16C294C5A41A47E15A2456F491EB38D9E5310EE10B3149`, `273650405FE1B260DD7F301A1CDF6E565AFF3282DD10928FE4B5B4055EB9D946`, and `ECB7B16BE7D13E4EEF56A2458EA5C1811C0A08379B5D56644A81BC839F5F4058`. They are video-only template evidence; the standalone NLE test separately exercises audio output. They are not an end-to-end vertical Shorts/audio certification.

## 5. Data safety, security, and privacy

Automated tests exercise project/profile/workspace serialization, migrations, future-schema handling, path validation, history/manifest sanitization, command construction, and recovery components. The saved-project integration test crosses project persistence, queue submission, real fixture rendering, and output creation. No deterministic data-loss or security defect was found in the inspected paths. This is not a penetration test or a guarantee against every crash/filesystem failure.

Opening a project does not intentionally execute host code; external tools are invoked as processes with argument-separated inputs in inspected paths. URL validation accepts HTTP(S), and notification configuration uses references/environment rather than storing plaintext credentials in the observed records. No external URL, notification provider, microphone upload, or network service was exercised. The application’s local media paths were tested with FFmpeg already installed in a shared development location; a clean first install and fully offline packaged workflow remain unverified.

Presentation Studio documentation states that autosave checkpoints after 20 seconds of UI idleness, retains up to 20 checkpoints, and requires explicit recovery. Edits made and the app closed before the first checkpoint can be lost. Corrupt checkpoints are ignored. No forced GUI crash/restart recovery drill was completed; this is a known operational limitation, not a passed crash certification.

### Packaging safety fix

The Windows packaging script previously recursively removed fixed `target/distribution` and `target/release-stage` directories. This certification changed it to accept an isolated `-OutputRoot`, validate that root remains within the repository and has no reparse ancestor, and refuse to replace existing output directories. A repeat invocation against an existing output was rejected and the existing ZIP SHA-256 remained unchanged. Release instructions now document the isolated output-root workflow. The package includes `docs/KNOWN_LIMITATIONS.md`.

## 6. Performance and reliability observations

- Warm CLI `--version` startup: seven-process sample median 91.17 ms, min 79.57 ms, max 94.64 ms. This excludes GUI initialization and is not an application-startup benchmark.
- Larp render, CPU/VM: 2.106 s for a 2 s, 320×180/6 fps render; this is a single short fixture, not throughput characterization.
- Sono-GCC cold/warm: see section 3; no speed advantage demonstrated.
- Long-duration playback, RAM/VRAM trend, extended recording drift, physical GPU load, worker/device leak soak, and repeated GUI startup were **not tested**.
- Clippy warnings remain. No performance threshold was inferred from noisy single-run timing.

## 7. Package and release-process evidence

A fresh development portable package was produced at:

`target/rc0-review-fe8dc1e/distribution/Sono-Renderer-0.1.0-windows-x86_64-portable.zip`

Size: 54,475,754 bytes. SHA-256: `021adbf9795aa3504745e255dc1a18626e06d2bdbe2e31d508db40ee673dfda0`. Its manifest explicitly says `dirty: true` and `reproducible: false`. A smoke test extracted it under paths containing spaces and Unicode, ran from a foreign working directory with a restricted PATH, and successfully exercised version/help/jobs/media-tool-info CLI flows. This was run on the developer machine, not a clean Windows VM, and did not launch the GUI or import/render media from the package.

The package script preserved the pre-existing `target/distribution` archive (SHA-256 `731badffaeae2387fef8009a2fc34346ea62cb65431ebe4865f74666426c2da1`) and now refuses output replacement. No installer was built. No signing identity was configured or verified. No install/upgrade/uninstall test was possible.

The release documentation requires a clean tagged source, matching release notes, formatting/lint/test/release builds, Windows resources, installer, portable and installer smoke tests, draft release, and rights review. This checkout fails those conditions. The asset pack redistribution rights have not been documented. The generated third-party notices contain 196 crate entries; 16 license-text files are absent locally and require review. These are publication/legal blockers, not implementation claims.

## 8. High-severity findings and remediation status

### Release blockers

1. **Dirty, untagged source:** 131 status entries, no matching `v0.1.0` tag, and no `release-notes/v0.1.0.md`. No stable RC can be certified from this source identity.
2. **Windows release tooling missing:** `rc.exe` and Inno `ISCC.exe` unavailable. The executable lacks version resource metadata; no installer exists.
3. **Rights/notice review incomplete:** asset-pack redistribution rights are undocumented; 16 dependency license texts are missing from local notice generation. Distribution must wait for review.
4. **Clean-machine GUI validation absent:** no clean Windows VM/host was available. Installed first-run, GUI interaction, media import/playback, and installer upgrade/uninstall remain unverified.
5. **Several optional advertised workflows lack end-to-end evidence:** speech model, physical microphone, yt-dlp, real-song BPM corpus, and physical GPU were unavailable. Do not present them as release-certified.

### Product-scope limitations requiring truthful release notes

The current audit documents incomplete generic Watch coverage, a non-spatial generic Chart Studio authoring scope, no live Watch-backed Presentation Studio source, and incomplete advanced NLE composition capabilities. The portrait template is functional but visually sparse. These limits do not invalidate the narrow Larp render test, but they do prevent broad “full Sonolus compatibility,” “complete Chart Studio,” or “professional NLE” claims. Exact feature caveats are maintained in [KNOWN_LIMITATIONS.md](../../KNOWN_LIMITATIONS.md).

### Fixed during this certification

- Release formatting gate: applied `cargo fmt --all`; formatting check now passes.
- Packaging safety: stopped recursive deletion of previous distribution/staging outputs; added bounded, validated output-root support and a refusal guard. Verified the guard leaves the old ZIP hash unchanged.
- Release documentation: added this certification and linked known limitations from the main documentation/install path.

No feature expansion or renderer semantic change was intentionally made. The package output safety fix is the only behavioral tooling change.

## 9. Release decision

**NOT READY FOR RELEASE CANDIDATE.** The renderer and several project/NLE paths build and pass meaningful automated and real-media checks. The direct Larp rendering path is reproducible against the prior audit fixture. However, release identity and source cleanliness are invalid, required Windows release tooling is missing, publication rights/notices are incomplete, and no clean-machine installed GUI certification exists. Product areas with missing hardware/model/network evidence must remain explicitly limited.

Before a future RC, the release owner must at minimum:

1. Review and commit the intended source, resolve warnings/known limitations as required by the release scope, and build from a clean version-matched tag.
2. Add matching `release-notes/v0.1.0.md` and rerun the documented release gates from that exact tag.
3. Provide `rc.exe`/Windows SDK and Inno Setup; embed version metadata, build the installer, and test install/upgrade/uninstall.
4. Complete asset and dependency license review, including missing license texts, and record redistribution decisions.
5. Test on a clean Windows machine/VM, including installed GUI first launch, standalone media import/playback/export, and direct render.
6. Either validate microphone/STT/yt-dlp/GPU on supported equipment or release with those workflows clearly marked optional/unverified and without implying certification.

This certification is evidence for a development snapshot only; it does not authorize publication.
