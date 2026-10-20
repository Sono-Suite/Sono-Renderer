# Follow-up #29 — URL media acquisition

## Scope and result

This pass adds a bounded URL-acquisition adapter that ends in a validated
local file and assigns that file to the renderer's existing `music` and `mv`
input references. It does not create a second media library, project schema,
download queue, or timeline clip type.

This is a **partial integration**, not completion of the full Follow-up #29
acceptance criterion. Repository inspection found that Chart Studio edits
generic Sonolus LevelData and can render a still Watch preview, but has no
audio playback, waveform, or music timeline. Presentation Studio is a still
frame compositor preview, not a nonlinear editor; there is no media bin,
trim/split timeline, proxy manager, or persistent video-editing project.
Accordingly, acquisition is reachable from Project / inputs and Chart Studio,
but the nonexistent playback/editor workflows were not represented as if
they had been implemented.

## Existing architecture and integration

- `RenderConfig.music` and `RenderConfig.mv` are ordinary file paths used by
  rendering. Project persistence stores corresponding `music` and `mv` input
  references. Acquisition assigns those fields and reuses project save/path
  behavior.
- `src/media_acquisition.rs` owns yt-dlp invocation, source inspection,
  progress parsing, cancellation, FFprobe validation, content hashing, and
  staging/finalization. It returns an ordinary `AcquiredMedia` path/result.
- `src/ffmpeg.rs` remains the owner of managed FFmpeg/FFprobe resolution. The
  acquisition process passes the resolved executable path explicitly and
  propagates cancellation through the existing export control.
- The render manifest continues to identify local media by content hash; the
  source URL is not needed to reproduce a render and is not persisted.

## Implemented workflow

- Desktop **Project / inputs** and **Chart Studio** provide Inspect URL,
  high-level mode selection, Download and assign, progress, and cancellation.
  Audio modes assign Music / BGM; video assigns MV; video + audio assigns both.
- CLI provides `media tool-info`, `media inspect-url`, and `media download`,
  with `--mode`, `--output-dir`, `--project`, and JSON output options.
- Supported modes are source-format audio, MP3 audio, MP4-compatible video,
  and MP4-compatible video + audio. FFmpeg is used for MP3 extraction and
  video stream merging/remuxing; no lossy video transcode is requested.
- Inspection exposes title, uploader, duration, live status, and available
  format count when yt-dlp provides them. A single HTTP(S) item is required;
  playlists and live/upcoming sources are refused.
- Downloads use a private same-destination staging directory and a generated
  filename. FFprobe checks container, positive duration, and required stream
  types before the file is atomically renamed into the destination. The
  finalized local bytes are SHA-256 hashed.
- Saved projects put acquired files in a `media/` folder next to the project;
  otherwise the per-user media directory is used. Project input references
  are saved using the existing project serialization. The URL itself is not
  persisted.
- Cancellation terminates the yt-dlp process tree and is propagated into
  managed FFmpeg provisioning/processing. Normal failure/cancellation drops
  the staging directory without changing project references.

## Dependency and safety policy

- Windows releases provision the official `yt-dlp.exe` only after the user
  explicitly inspects or downloads a URL. `media tool-info` never downloads
  it. The managed version is pinned at `2026.08.19` with SHA-256
  `66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a`, then
  version-checked before execution. Updates occur with a renderer release;
  there is no silent self-update, PATH discovery, user-selected executable,
  or bundling in the app package.
- Provisioning uses the Windows system `curl.exe`, HTTPS-only, into a staging
  directory. No download is executed until size, SHA-256, and version checks
  pass. The resolver is currently Windows-only.
- yt-dlp is launched with structured process arguments, no shell, `--` before
  the URL, ignored config, disabled plugin directories, and single-item mode.
  Browser cookies, credentials, and authentication headers are not supplied.
  DRM/access-control circumvention is not implemented.
- Metadata and process output are bounded. Diagnostics redact URL query and
  fragment data. Titles/uploader names are display metadata only; generated
  local filenames do not derive from remote text.
- Ordinary local-file rendering/import remains independent of yt-dlp.

## Verification

### Build and package checks — passed

- `cargo fmt --all` and `cargo fmt --all -- --check`
- `cargo check --locked`
- `cargo clippy --all-targets --locked` (completed; the repository reports a
  substantial existing warning backlog)
- `cargo build --release --locked --bin renderer`
- `scripts/package-windows.ps1 -SkipBuild`
- `scripts/smoke-test-windows-package.ps1 -DistributionDirectory target/distribution -PortableOnly`

The portable package smoke test extracted the packaged application under a
Unicode/space-containing path, ran it from a foreign working directory, and
checked version/help/jobs plus `media tool-info --json`. It verified that a
clean portable profile reports the pinned yt-dlp dependency as unavailable
without attempting a download. The package contains the acquisition guide.
The tested portable ZIP SHA-256 is
`731badffaeae2387fef8009a2fc34346ea62cb65431ebe4865f74666426c2da1`.
Inno Setup 6 was unavailable, so this environment produced and tested only
the portable ZIP, not an installer.

### Tests — passed with limitations

- Media acquisition unit tests: 8 passed, covering URL validation, playlist
  and live metadata classification, mode requirements, safe argument
  construction, progress parsing, redaction, bounded output, and cancellation
  propagation to the FFmpeg resolver.
- CLI parsing and project assignment tests: 2 passed, including saving and
  reopening existing project `music`/`mv` references.
- Before the final small Chart Studio UI placement and FFmpeg cancellation
  wiring, segmented serialized repository tests passed: 290 library tests
  (4 ignored), 27 binary tests, and 113 integration tests. The cancellation
  change also passed its targeted test and the FFmpeg module test.
- A parallel full `cargo test` attempt terminated a Windows test executable
  with status `0xc0000005`; the serialized test targets above passed. This
  environment-dependent failure remains recorded rather than hidden.

## Not tested / intentionally deferred

- No authorized media URL was supplied, and the managed yt-dlp dependency was
  absent. There was no real online inspection/download, actual media decode,
  real-media MP3 timing test, or network-failure exercise.
- The portable smoke test tested dependency status and CLI wiring, not an
  actual packaged acquisition. No installer was available for validation.
- GUI controls were build-checked but not interactively exercised.
- Chart Studio waveform generation, music playback/scrubbing, MP3
  encoder-delay alignment, audio-synchronized Watch preview, and full
  chart-authoring round trip are unavailable in the current Chart Studio.
- Presentation Studio media-bin import, timeline editing, trimming, audio
  mixing, proxy/thumbnail workflows, and final NLE export are unavailable in
  the current product. Acquired video only feeds the existing MV background
  input; its source audio is not mixed into renderer output.
- Playlist item selection, configurable download size/duration limits,
  concurrent download scheduling across separate CLI processes, retries,
  source provenance, remembered-source relinking, thumbnails, and proxies
  are not implemented.
- FFprobe validates container, duration, and stream presence. This pass does
  not establish decoded seek/frame-step behavior, timestamp integrity, codec
  compatibility across all downstream consumers, or MP3 playback alignment.
- No new acquisition-history system was added. Existing render history and
  manifests continue to describe render attempts and local content identity.

## Reproduction commands

```powershell
cargo test --lib media_acquisition::tests -- --test-threads=1
cargo test --bin renderer media_acquisition -- --test-threads=1
cargo test --bin renderer acquired_files_use_existing_project_music_and_mv_references -- --test-threads=1
cargo build --release --locked --bin renderer
.\scripts\package-windows.ps1 -SkipBuild
New-Item -ItemType Directory -Force -Path 'target\release-smoke-temp' | Out-Null
$env:RUNNER_TEMP = (Resolve-Path 'target\release-smoke-temp').Path
.\scripts\smoke-test-windows-package.ps1 -DistributionDirectory target/distribution -PortableOnly
```
