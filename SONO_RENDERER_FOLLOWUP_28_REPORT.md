# Sono-Renderer Follow-up #28: release and distribution report

## Release outcome

The repository now has one Windows x86_64 packaging path for the existing
renderer binary, a per-user Inno Setup installer definition, a portable ZIP,
build identity, checksums, a release manifest, user installation guides, and a
tag-triggered GitHub Actions workflow. The workflow creates a **draft** GitHub
release for maintainer review.

This is not yet a production-approved public release. The local machine lacks
Inno Setup and the Windows SDK resource compiler, so I built and exercised the
portable package only. No official release workflow was run. The repository
does not record redistribution rights for its art/media assets, and the notice
generator reports Cargo crates whose registry archives omit their license
text. The draft must not be made public until those items and the clean-machine
GUI/render acceptance checks are complete. Windows Authenticode signing is not
configured.

## Release architecture added

- `Cargo.toml` is the sole semantic version source. `build.rs` embeds the short
  Git commit, dirty flag, build channel, target, profile, and build ID. The CLI
  `--version` and desktop About view use this identity. On Windows, the same
  build script embeds PE version resources through `rc.exe`; stable packaging
  fails if those resources cannot be generated.
- `scripts/package-windows.ps1` builds the existing `renderer` executable once
  and stages that binary with the supplied `assets`, UI/watermark profiles,
  notices, and concise user guides. It creates a portable ZIP and, when Inno
  Setup 6 is installed, a per-user installer. It writes a JSON manifest and
  SHA-256 list under ignored `target/distribution`.
- `packaging/windows/Sono-Renderer.iss` installs to
  `%LOCALAPPDATA%\Programs\Sono-Renderer`, creates a Start menu shortcut and
  optional desktop shortcut, and registers an App Paths entry. It has no file
  associations and does not put user data in the installation directory.
- Windows portable mode requires the explicit versioned `portable.mode`
  marker. It stores configuration, profiles, jobs/history, performance data,
  diagnostics, and caches below `UserData`; installed builds use
  `%LOCALAPPDATA%\Sono-Renderer`. Projects, source media, and outputs remain
  at user-selected locations.
- `.cargo/config.toml` statically links the MSVC CRT so an extra Visual C++
  redistributable is not an application prerequisite. A working GPU driver and
  Windows system libraries remain machine requirements.
- `.github/workflows/release-windows.yml` accepts matching clean `v<version>`
  tags with tag-specific human notes. It runs formatting, Clippy, tests,
  packaging, portable and installer smoke tests, generates GitHub artifact
  provenance, and creates a draft release. It does not auto-publish the draft.

## FFmpeg, GPU, and Sono-GCC

FFmpeg is not in either package. On Windows, the first MP4 export checks an
application-local `addons` directory and the per-user managed install, then
downloads the pinned Gyan 8.1 full-build archive, verifies its SHA-256, checks
the `ffmpeg`/`ffprobe` pair, and stores it in the per-user data directory. A
first MP4 export therefore needs network access if FFmpeg is not already
installed. The resolver deliberately does not use an arbitrary executable
from `PATH`.

Gyan describes its Windows builds as static GPLv3 builds and requires Windows
10 or later for current packages. FFmpeg's own legal page explains that the
upstream project is LGPL by default, while enabling optional GPL components
changes the license that applies to the built binary. Sono-Renderer does not
redistribute the downloaded binary; maintainers must still review the exact
pinned 8.1 archive before changing that policy or bundling FFmpeg in the
future. See [Gyan's Windows build page](https://www.gyan.dev/ffmpeg/builds/)
and [FFmpeg's licensing notes](https://www.ffmpeg.org/legal.html).

The application uses WGPU and includes D3D12, GLES, and Vulkan backends. A
working adapter/driver is expected for GPU rendering; the existing CPU backend
is available in renderer settings. I did not open the packaged GUI on this
machine.

Sono-GCC remains optional. It is not bundled and needs `rustc` from `RUSTC` or
`PATH` to compile eligible Watch regions. VM execution remains the default and
the correctness reference; unavailable or failed native compilation follows
the existing VM fallback policy. Generated DLLs may cause security software
alerts; users should not disable their security controls to use the VM.

## Persistence, update, and trust behavior

- Inno upgrades use the same application identity/install folder while user
  state remains outside that folder. Same-version reinstall and uninstall
  preservation are automated in the CI smoke script but could not be exercised
  locally without Inno Setup.
- The installer uninstaller removes the application files, shortcuts, and App
  Paths registration. It deliberately leaves `%LOCALAPPDATA%\Sono-Renderer`
  and all external projects/media untouched. Portable updates are manual:
  copy `UserData` to a new extraction and keep the old folder until verified.
- There is no in-app updater, signed update manifest, staged update, or
  automatic rollback. Manual rollback means installing a previously retained
  setup package; app data is kept. An interrupted-update scenario was not run.
- The release workflow creates an unsigned draft with SHA-256 checksums and
  GitHub build-provenance attestations. Checksums detect corruption only when
  compared with a trusted copy; attestations do not replace Authenticode
  signing. The draft requires explicit human review before public release.
- The installer creates no `.sonoproject` file association. Projects are
  opened in the GUI. Jobs recover through the existing queue/history stores;
  there is no app-wide crash dump, persistent log bundle, or safe-mode
  launcher.
- Structured render diagnostics can be inspected/exported using existing
  queue/history and CLI workflows. Diagnostic export has tests for credential
  and path redaction. No automatic diagnostic upload is configured.

The package includes the code `LICENSE` and generated `THIRD_PARTY_NOTICES.md`.
The notice script reads the Windows-target locked Cargo dependency set and
includes available registry license/notice files. If an archive has an SPDX
license expression but omits its text file, the notice calls that out with the
upstream repository rather than claiming the text was included. In this
checkout it found 194 registry packages; some lack included texts and need
review. More significantly, the 199 supplied media files in `assets/` have no
redistribution terms recorded here. The release script packages them because
they are required visual resources, but no one should publish those artifacts
until the asset rights and attributions are verified.

## Validation performed

Baseline before release changes: `cargo test --locked --offline
--no-fail-fast` passed **420**, with **4 ignored**. After implementation, the
debug suite passed **421**, with **4 ignored**. The optimized release suite
`cargo test --release --locked --no-fail-fast` also passed **421**, with **4
ignored**. This includes tests for real fixtures, Watch execution, VM/GCC
parity and fallback, rendering pipelines, chart export/runtime handoff,
profiles, persistence, diagnostics, and watermark invariants.

`cargo fmt --all -- --check` passed. `cargo clippy --locked --all-targets`
completed with exit code 0 and 176 warning records across the existing broad
codebase; warnings are not promoted to errors. The Windows release binary
built successfully in release mode. Local PE version resources were omitted
because `rc.exe` is unavailable; the stable packaging gate requires them.

The development-channel portable archive was built at
`target/distribution/Sono-Renderer-0.1.0-windows-x86_64-portable.zip` (53,238,744
bytes). SHA-256:

```text
bc0558fb9a0621a892c1cffa9c48e6f518bd2b4c49e0883ec156b8f107bfd3fc
```

The generated manifest reports `dirty: true`, channel `development`, and
`reproducible_build: false`. It is a locally inspectable test artifact, not a
stable or signed release. The package smoke test extracted it under a path with
spaces and a non-ASCII character, launched it from `C:\Windows\System32` with
`PATH` restricted to Windows system binaries, and exercised `--version`,
`--help`, and `jobs list`. It verified the portable marker, packaged asset
manifest, `UserData\jobs.json` path, and packaged presentation guide. An
invalid engine CLI request returned exit code 1. PowerShell syntax parsing
passed for all three packaging scripts.

## 70-item acceptance report

Status terms describe evidence: **implemented** means code/configuration is
present; **unit/integration-tested** means exercised by the test suite;
**packaged-tested** means exercised from the generated archive; **build-checked**
means compiled locally; **unavailable/not run** means the environment or tool
was absent; **deferred** means the product has no such feature.

| # | Area | Status and evidence |
|---:|---|---|
| 1 | Existing release architecture | **Manually inspected:** one Cargo package, one library, one GUI/CLI executable; no previous installer workflow. |
| 2 | Supported platform assessment | **Implemented/build-checked:** Windows x86_64 path. Linux and macOS have no release build or package. |
| 3 | Windows installer | **Implemented:** Inno per-user definition and CI smoke path. **Unavailable locally:** ISCC.exe absent. |
| 4 | Portable Windows build | **Implemented, build-checked, packaged-tested:** ZIP generated and CLI smoke passed. |
| 5 | Linux packaging | **Deferred:** no Linux distribution format or CI job. |
| 6 | macOS | **Deferred:** no macOS build or package. |
| 7 | CPU architecture | **Implemented:** x86_64 Windows target only; ARM64 is not packaged. |
| 8 | Version source | **Implemented/build-checked:** `Cargo.toml` package version is canonical. |
| 9 | Build identity | **Implemented/build-checked/packaged-tested:** CLI identity tested; PE resource path requires Windows SDK `rc.exe`. |
| 10 | Release channel | **Implemented:** development/stable distinction; stable requires a clean matching tag and PE metadata. Stable path not run. |
| 11 | FFmpeg distribution | **Implemented/manually inspected:** lazy pinned per-user download, SHA-256 verification, no PATH search; no local FFmpeg. |
| 12 | FFmpeg licensing | **Manually inspected:** exact dependency is external Gyan 8.1 full build; current Gyan page labels Windows builds static GPLv3, and FFmpeg explains build-option licensing. Exact pinned archive still requires maintainer review. |
| 13 | GPU requirements | **Build-checked/documented:** WGPU D3D12/GLES/Vulkan features and existing CPU backend; GUI/driver behavior not tested. |
| 14 | Sono-GCC packaged runtime | **Implemented/documented:** no compiler bundled; VM is the default fallback. Packaged GCC not tested. |
| 15 | Sono-GCC compiler strategy | **Manually inspected:** optional `rustc` from `RUSTC`/`PATH`; no install-time compiler requirement. |
| 16 | Sono-GCC cache/version | **Unit-tested/documented:** current cache identity includes compiler/runtime/target data. |
| 17 | Sono-GCC security software | **Documented:** generated native DLLs may be flagged; guidance retains the VM option. No antivirus suite tested. |
| 18 | Application filesystem layout | **Implemented/unit-tested:** installed binaries under per-user Programs; portable resources beside executable. |
| 19 | User-data layout | **Implemented/unit-tested/packaged-tested:** AppData for install, explicit `UserData` portable marker for ZIP. |
| 20 | Install behavior | **Implemented:** per-user, no administrator prompt. **Unavailable locally:** installer absent. |
| 21 | Upgrade behavior | **Implemented/documented:** same Inno app identity and external user data. No older-version upgrade run. |
| 22 | Persistent-data migrations | **Unit-tested by subsystem:** versioned stores/migrations exist. Cross-release migration not tested. |
| 23 | Newer project with older app | **Unit-tested by schema owners:** unsupported future schemas are rejected or isolated where implemented; no cross-version app test. |
| 24 | Update system | **Deferred:** no automatic in-app updater. |
| 25 | Update trust/integrity | **Implemented:** SHA-256 list and workflow provenance; no signed updater manifest or Authenticode signature. |
| 26 | Rollback | **Documented manual path:** reinstall a retained old installer; no automatic rollback. |
| 27 | Offline behavior | **Partially verified:** CLI package smoke needed no external PATH tools. Full offline render and FFmpeg download failure not run. |
| 28 | Repair behavior | **Implemented in CI smoke script:** same-version reinstall; **not locally run** without Inno. |
| 29 | Uninstall behavior | **Implemented in Inno definition:** removes app/registration and leaves user data. **Not locally run.** |
| 30 | File associations | **Deferred:** no project or media association is registered. |
| 31 | Process lifetime/shutdown | **Manually inspected:** standard desktop process, no updater service or tray process added. GUI shutdown not manually exercised. |
| 32 | Crash handling | **Manually inspected/documented:** no automatic crash upload or app-wide crash dump. |
| 33 | Diagnostic bundle | **Deferred:** structured render diagnostics exist; there is no single app-wide support bundle. |
| 34 | Privacy/redaction | **Unit-tested:** diagnostic export credential/path redaction. No support bundle is uploaded automatically. |
| 35 | Safe/recovery startup | **Partially implemented/unit-tested:** queue recovery exists; no app-wide safe-mode launcher. |
| 36 | Log management | **Manually inspected:** no managed persistent application log; render diagnostics/history are the available records. |
| 37 | About/version UI | **Implemented/build-checked:** desktop About section displays version/build identity. **Not manually opened.** |
| 38 | Project license packaging | **Implemented/packaged-tested:** root MIT `LICENSE` is staged; package contents were inspected. |
| 39 | Third-party notices | **Implemented/build-checked:** Windows-target Cargo notices generated. Missing upstream license texts are called out, not represented as included. |
| 40 | Release-content audit | **Partially implemented:** asset/profile/docs staging is explicit; asset redistribution rights remain unknown and block public release. |
| 41 | Artifact size | **Packaged-tested:** portable ZIP is 53,238,841 bytes. No installer size available locally. |
| 42 | Code signing | **Deferred:** no Authenticode certificate or signed installer. |
| 43 | Reproducibility | **Explicitly not claimed:** manifest sets `reproducible_build: false`; toolchain/packaging image not fully pinned. |
| 44 | Checksums | **Packaged-tested:** generated SHA-256 matches an independent `Get-FileHash` result. |
| 45 | Release manifest | **Implemented/packaged-tested:** JSON includes version/channel/commit/target, artifact size, and SHA-256. |
| 46 | CI/release automation | **Implemented, not CI-tested:** clean-tag workflow builds, tests, attests, then creates a draft. |
| 47 | Packaged-artifact testing | **Packaged-tested:** portable extraction, asset presence, version/help/jobs CLI, restricted PATH, different working directory. |
| 48 | Clean-machine install | **Unavailable/not run:** no clean VM and no locally built installer. |
| 49 | Clean-machine first run | **Unavailable/not run:** GUI/onboarding not opened. |
| 50 | Clean-machine Quick Render | **Unavailable/not run:** no FFmpeg executable and no network download run; valid MP4 not produced. |
| 51 | Chart Studio | **Unit/integration-tested:** editor, canonical LevelData, and runtime handoff tests pass. Packaged interactive UI not tested. |
| 52 | Packaged `level.json.gz` export | **Unavailable/not run:** no packaged GUI session. Existing chart export tests pass. |
| 53 | Exported level through normal renderer | **Unit/integration-tested:** chart export/runtime handoff and real fixture tests pass; no packaged end-to-end export. |
| 54 | Presentation Studio | **Documented/partially implemented:** gameplay HUD profiles and still Program Preview exist; multi-track NLE is not present. |
| 55 | Full end-to-end acceptance | **Unavailable/not run:** clean install through render, chart export/reimport, presentation, update, diagnostics, and uninstall not performed. |
| 56 | Packaged Sono-GCC | **Unavailable/not run:** no package GCC run without a packaged GUI/render fixture; compiler is not bundled. |
| 57 | Offline acceptance | **Partially tested:** restricted-PATH CLI succeeded. Network-disabled GUI/render workflow was not tested. |
| 58 | Unicode/path behavior | **Packaged-tested:** executable extracted under Unicode/space path; CLI launched from another directory. Full project import/export path not run. |
| 59 | Upgrade test | **Unavailable locally:** CI script covers same-version reinstall only; no older representative package exists. |
| 60 | Failed-update test | **Deferred:** there is no updater to interrupt or roll back. |
| 61 | Uninstall/reinstall | **Implemented in CI smoke script, not run locally:** checks binary removal and preservation of AppData/project files. |
| 62 | Disk-space/error cases | **Not run:** install, cache, LevelData, and MP4 low-disk scenarios remain unverified. |
| 63 | Release performance | **Not measured:** build duration is not a render benchmark; no installed-machine workload baseline. |
| 64 | Release regression | **Release integration suite passed:** VM/GCC parity, Watch, GPU pipeline, export, and other fixture tests passed. Dedicated visual regression capture was not run. |
| 65 | Packaged CLI | **Packaged-tested:** `--version`, `--help`, and `jobs list` pass; invalid engine identity returns exit code 1. |
| 66 | Environment/path audit | **Partially tested:** package CLI works without developer PATH tools and from foreign CWD. GUI resource dialogs and external paths not tested. |
| 67 | User documentation | **Implemented/packaged-tested:** install, first render, troubleshooting, Chart Studio, gameplay presentation, and asset guides ship in ZIP. |
| 68 | Release checklist | **Implemented:** maintainer checklist and human-reviewed release-note template added. |
| 69 | Known production limitations | **Documented:** rights review, missing crate texts, unsigned Windows artifacts, no auto updater, no NLE, no Linux/macOS packages, and no clean-machine GUI acceptance. |
| 70 | Intentionally deferred work | **Deferred:** Linux/macOS/ARM64 releases, automatic updater/rollback, Authenticode, file associations, crash bundle/safe mode, NLE, and unverified asset redistribution. |

## Files to inspect

- [Installation guide](docs/INSTALLATION.md)
- [Release process and checklist](docs/RELEASE_PROCESS.md)
- [Getting Started](docs/GETTING_STARTED.md)
- [Chart Studio guide](docs/CHART_STUDIO.md)
- [Gameplay presentation guide](docs/PRESENTATION_STUDIO.md)
- [Windows release workflow](.github/workflows/release-windows.yml)
- [Windows package builder](scripts/package-windows.ps1)
- [Portable/installer smoke test](scripts/smoke-test-windows-package.ps1)
- [Generated portable archive](target/distribution/Sono-Renderer-0.1.0-windows-x86_64-portable.zip)
- [Release manifest](target/distribution/Sono-Renderer-0.1.0-release-manifest.json)
- [SHA-256 list](target/distribution/SHA256SUMS.txt)
