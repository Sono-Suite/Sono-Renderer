# URL media acquisition

The desktop **Project / inputs** panel, **Chart Studio**, and CLI can acquire
one authorized HTTP(S) media item through the official yt-dlp executable.
The downloaded result is an ordinary local file assigned to the existing
project or render input fields; there is no second media library or
acquisition queue.

## Desktop workflow

1. Open **Project / inputs** or **Chart Studio**, then expand **Acquire media
   from URL**.
2. Paste one HTTP(S) URL and choose **Inspect URL**. The first inspection
   installs the pinned yt-dlp Windows executable into per-user application
   data after checking its release SHA-256 and version.
3. Review the available title/uploader/duration information and choose
   **Download and assign**. The application chooses an opaque generated file
   name, validates the result with the same FFprobe pair used by export, then
   publishes the file into `media/` beside a saved project. Without a saved
   project, it uses the renderer's per-user `media` directory.
4. Audio modes set the existing Music / BGM input. Video sets the existing MV
   input. Video + audio sets both references to the same local container.
   Save an unsaved project to retain the references.

The modes are **Audio (source format)**, **Audio (MP3)**, **Video
(MP4-compatible)**, and **Video + audio (MP4-compatible)**. MP3 extraction,
stream merging, and MP4 remuxing use the existing managed FFmpeg/FFprobe
installation. Media is remuxed when the selected mode requests MP4 and the
source allows it; Sono-Renderer does not request lossy video re-encoding.
Audio source format is retained unless MP3 is explicitly selected.

## CLI

```powershell
Sono-Renderer.exe media tool-info
Sono-Renderer.exe media inspect-url 'https://example.test/item' --json
Sono-Renderer.exe media download 'https://example.test/item' --mode audio
Sono-Renderer.exe media download 'https://example.test/item' --mode audio-mp3 --project chart.sonoproject
Sono-Renderer.exe media download 'https://example.test/item' --mode video-audio --project chart.sonoproject --json
```

Without `--output-dir`, CLI downloads go to `media/` beside `--project`, or to
the renderer's per-user media directory when no project is supplied. Project
assignment updates the existing `music` and/or `mv` references with an atomic
project save. JSON output includes the final local path, stream flags, duration,
format, and content SHA-256. It does not include the source URL.

## Safety, privacy, and offline behavior

- Only one HTTP(S) URL is accepted. Embedded URL credentials, playlists,
  upcoming streams, and live streams are refused. `--no-playlist` is passed
  even after metadata inspection.
- The managed executable is pinned to the upstream `2026.08.19` Windows
  release and its SHA-256. It is installed only after an explicit URL
  inspection/download action; it is not silently updated during rendering.
  A later renderer release may pin a newer version while retaining the prior
  versioned file. `media tool-info` does not install or update anything.
- The application does not bundle or redistribute yt-dlp. It downloads the
  official Windows executable on request into app data. The official project
  notes that its PyInstaller binaries include GPLv3+ components and publishes
  release checksums and signatures; review its current terms and notices before
  changing the deployment strategy:
  [yt-dlp releases](https://github.com/yt-dlp/yt-dlp/releases/tag/2026.08.19),
  [yt-dlp README and license notes](https://github.com/yt-dlp/yt-dlp#license).
- Extractor configuration files and plugin directories are disabled. Sono-
  Renderer does not pass browser cookies, credentials, or authentication
  headers and does not implement access-control, DRM, paywall, or geographic
  restriction circumvention.
- Source URLs are not stored in project files, render manifests, history, or
  ordinary command output. Error output is bounded and URL query/fragment
  components are redacted. The selected local file's content hash is the
  identity used by the existing render manifest; a URL alone is not treated as
  reproducible identity.
- Once finalized, the file is local and works offline wherever the project
  reference resolves. No URL is revisited to open a project or reproduce a
  render. Missing local media must be selected or reacquired explicitly.
- Acquisition uses a private same-filesystem staging directory, bounded
  process output, cancellable yt-dlp/FFmpeg child processing, FFprobe stream
  validation, a generated filename, and no-overwrite atomic publication.
  Partial staging files are removed on normal failure/cancellation.
- The Windows release provisions yt-dlp using the Windows system `curl.exe`
  over HTTPS. Other platforms are not supported by the managed acquisition
  provider in this release. An unavailable network or system curl prevents new
  URL acquisition but does not affect local media import or rendering.

## Current renderer boundaries

Acquired audio can be selected as render BGM and goes through the existing
FFmpeg audio preparation path. The Chart Studio acquisition panel assigns the
audio to that same project Music / BGM input; Chart Studio still edits generic
LevelData and renders a still Watch frame, with no audio playback, waveform, or
music timeline. Acquired video can be selected as the existing custom MV
background and is sampled by the production compositor; its source audio is
not mixed into the gameplay render. Presentation Studio is a separate local
media composition workflow; it can import acquired files from disk, but URL
acquisition does not itself create a composition, timeline clip, or proxy.
See [Presentation Studio](PRESENTATION_STUDIO.md).

## Troubleshooting

- **yt-dlp install/hash/version check failed:** retry on a network that can
  reach the official GitHub release. The application does not run a binary
  whose hash or version differs from the pinned release.
- **FFmpeg/FFprobe is unavailable:** MP3, video processing, and final media
  validation require the existing renderer-managed FFmpeg pair. Resolve the
  standard FFmpeg export dependency and retry.
- **URL/source failure:** the technical error is shown without intentionally
  retaining query tokens. Extractor sites change; Sono-Renderer does not infer
  a more specific cause from unstable error strings.
- **Playlist or live source:** use a direct, non-live item URL. Playlist
  selection and live capture are not implemented.
- **Offline:** local media and renders remain available. New URL inspection or
  acquisition requires network access when the pinned executable is absent.
