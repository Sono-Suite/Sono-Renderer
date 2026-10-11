# Install and run Sono-Renderer

## Platform status

- **Windows x86_64:** the release workflow is configured to produce a per-user
  installer and a portable ZIP. Its CI smoke path covers CLI launch, portable
  extraction, installation, same-version reinstall, and uninstall. A clean
  machine GUI acceptance run is still required before calling a release fully
  production-validated.
- **Linux:** source contains Linux UI and graphics paths, but this release
  workflow does not build or test Linux packages. Treat Linux as unverified.
- **macOS:** no build or package is produced; macOS is unsupported by this
  release workflow.
- **Architecture:** Windows x86_64 only. Windows ARM64 is not packaged.

No public release artifact has been approved from this checkout yet. The
workflow creates a draft release for review; use the download steps below
after maintainers publish an approved release.

## Download and install

1. Download the Windows x86_64 setup executable from the project's official
   GitHub Release page and verify its SHA-256 against `SHA256SUMS.txt`.
2. Run the setup executable. It installs for the current user under
   `%LOCALAPPDATA%\Programs\Sono-Renderer`; no administrator account is
   required. A Start menu shortcut is created. A desktop shortcut is optional.
3. Launch **Sono-Renderer**. Choose **Quick Render**, select an engine ZIP,
   level, resource package, optional music, and an MP4 output, then check the
   inputs and queue the render. A project or profile is optional.

The release archive includes the executable, the supplied `assets`, UI and
watermark profiles, the project license, third-party notices, and short user
guides. It does not include source fixtures, test suites, caches, projects,
compiler toolchains, or FFmpeg binaries.

## Portable ZIP

Extract `Sono-Renderer-<version>-windows-x86_64-portable.zip` to a writable
folder and run `Sono-Renderer.exe`. The included, versioned `portable.mode`
marker explicitly enables portable storage. Configuration, profiles, queue,
history, performance records, and cache are kept under the extracted folder's
`UserData` directory, regardless of the current working directory. Back up
`UserData` before deleting or moving the extracted folder. Projects and media
remain at their referenced paths and are not copied into the archive.

The portable package is not an installer and does not create shortcuts or
Windows application registration.

## FFmpeg and offline use

The setup and portable packages do not redistribute FFmpeg. On Windows, the
first MP4 export checks an app-local `addons` directory, then the user's
managed FFmpeg 8.1 installation. If neither contains a matching `ffmpeg` and
`ffprobe` pair, Sono-Renderer downloads the pinned Gyan 8.1 full-build ZIP,
checks its SHA-256, validates both executables, and stores them in:

```text
%LOCALAPPDATA%\Sono-Renderer\dependencies\ffmpeg\8.1
```

The first MP4 export therefore needs an internet connection; other local
operations do not require an update service or account. If the download fails,
the app reports the required version, upstream URL, and technical cause.
`Sono-Renderer.exe ffmpeg-info` inspects an already-present pair without
triggering a download.

The resolver does not search `PATH` and does not select arbitrary executables.
The Gyan page describes its Windows builds as static GPLv3 builds. The app does
not bundle or repackage that third-party binary; its exact package and license
are controlled by the upstream distribution. Review the upstream build page
before changing the pinned distribution or including FFmpeg in a future
release. The SHA-256 pin protects the downloaded archive's identity; it is not
a signature from Sono-Renderer.

Gyan documents its current Windows builds as requiring Windows 10 or later.
Older Windows versions have not been tested for media export.

## URL media acquisition

yt-dlp is optional and is not included in the installer or portable ZIP. On
the first explicit URL inspection, Sono-Renderer downloads the pinned official
Windows executable into the same per-user/portable `UserData` root used by the
application, validates its release SHA-256 and version, and reuses that
versioned copy. This requires the network and Windows system `curl.exe`.
Renderer updates control the pinned yt-dlp version; no tool self-update runs
during a render. MP3/video acquisition also resolves the existing FFmpeg pair.
Local file selection, project opening, and rendering do not require yt-dlp or
network access. See [URL media acquisition](MEDIA_ACQUISITION.md) for current
support and limitations.

## GPU and CPU rendering

The default render backend uses WGPU and needs a working graphics adapter and
driver. If initialization fails, the error contains the adapter/device cause;
select the existing **CPU** backend in renderer settings to render without a
GPU. CPU rendering can be substantially slower. The installer does not bundle
graphics drivers.

## Sono-GCC

Sono-GCC is optional and is not needed for ordinary use. The Sono VM remains
the semantic reference and can execute the renderer without a Rust compiler.
On Windows, native Sono-GCC compilation generates Rust code, invokes `rustc`
from `RUSTC` or `PATH`, builds a DLL, and loads it into the current process.
The compiler is not bundled. If it is missing, blocked, or cannot compile/load
a region, the existing execution policy uses the VM where supported. Security
software can flag generated native DLLs; do not weaken security controls to
enable it. GCC cache files are derived data and can be removed through Cache
Management.

## User data, upgrades, rollback, and uninstall

Installed Windows builds keep application data under
`%LOCALAPPDATA%\Sono-Renderer`; the binaries and supplied read-only examples
are under `%LOCALAPPDATA%\Programs\Sono-Renderer`. Projects/media are stored
where the user chooses. Updating by running a newer setup package preserves
the app-data directory. There is no automatic in-app updater: check the
official release page and explicitly install a verified release. To roll
back, close the app and reinstall a previous setup package. The same version
can be reinstalled to restore missing application files.

Uninstall removes installed binaries, shortcuts, and the per-user App Paths
registration. It intentionally retains `%LOCALAPPDATA%\Sono-Renderer`, which
contains configuration, profiles, queue/history, diagnostics/performance
records, and disposable caches. To remove it, review and delete that directory
yourself after uninstall. External project, chart, media, and render files are
not managed by the uninstaller.

The portable ZIP has no updater or uninstaller. To update, back up `UserData`,
extract the new version to a separate folder, then copy `UserData` into it.
Keep the old folder until the new version opens your work successfully.

## Diagnostics and privacy

Render errors and their structured diagnostics are available in the render
queue/history and via the existing `diagnostics` CLI commands. Diagnostic
export sanitizes credential-like values and user paths where implemented.
Sono-Renderer does not automatically upload crash reports. There is no
application-wide crash dump/log bundle or safe-mode launcher in this release;
preserve the displayed error and the relevant render diagnostic when asking
for help. Inspect the bundle's contents before sharing it.

## CLI

The installer and portable archive contain the same executable used for the
GUI. In PowerShell, run:

```powershell
& "$env:LOCALAPPDATA\Programs\Sono-Renderer\Sono-Renderer.exe" --version
& "$env:LOCALAPPDATA\Programs\Sono-Renderer\Sono-Renderer.exe" --help
```

For a portable installation, run `Sono-Renderer.exe --help` from its extracted
folder. The renderer accepts paths with spaces and Unicode. There is no
`.sonoproject` file association yet; open projects from the GUI.

## Third-party licenses and signing

The application license is in `LICENSE`; release packages include
`THIRD_PARTY_NOTICES.md` generated from the locked Cargo dependency set. The
notice file does not cover the artwork and media in the repository's
`assets/` directory. Their distribution terms are not recorded in this
repository, so the included art pack requires a rights review before a public
release. The FFmpeg executable is downloaded separately as described above. Windows
release executables are not code-signed in the current release setup. GitHub
Actions publishes build-provenance attestations for release artifacts, but
these do not replace a Windows Authenticode signature. Windows may show an
unknown-publisher warning until official code signing is configured.

See [Getting Started](GETTING_STARTED.md), [Troubleshooting](TROUBLESHOOTING.md),
[Chart Studio](CHART_STUDIO.md), [gameplay presentation and preview](PRESENTATION_STUDIO.md),
and [Presentation Studio asset profiles](PRESENTATION_STUDIO_ASSETS.md) for
specific workflows. See [Current limitations](KNOWN_LIMITATIONS.md) before
relying on optional or platform-limited capabilities.
