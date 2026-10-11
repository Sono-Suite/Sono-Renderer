# Troubleshooting

## The application will not start

Use the Windows x86_64 package on a supported Windows system. Reinstall from
the setup package to restore application files; this leaves application data
and projects intact. In a PowerShell window, check the executable directly:

```powershell
$renderer = Join-Path $env:LOCALAPPDATA 'Programs\Sono-Renderer\Sono-Renderer.exe'
& $renderer --version
& $renderer --help
```

For a portable copy, set `$renderer` to the extracted `Sono-Renderer.exe`.
Sono-Renderer does not create an application-wide crash dump or persistent log
bundle. Preserve the full displayed error and associated render diagnostic
when asking for help.

## GPU initialization fails

Install or update the graphics driver provided by the computer or GPU maker,
then restart Sono-Renderer. If GPU rendering still fails, select the existing
**CPU** backend in renderer settings. CPU rendering can be slower. The
installer does not install or update graphics drivers.

## FFmpeg is unavailable

MP4 export needs a matching `ffmpeg` and `ffprobe` pair. `& $renderer ffmpeg-info`
checks for an existing pair without downloading one. On Windows,
the first MP4 export downloads the pinned Gyan FFmpeg 8.1 package only when a
validated pair is not already beside the executable or in the managed
per-user location. Check network access and that
`%LOCALAPPDATA%\Sono-Renderer` is writable, then retry. The app does not
search `PATH` for an arbitrary FFmpeg executable. Non-Windows export paths are
not packaged or verified in this release.

## A render fails

Confirm that the engine ZIP, level, and resource package belong together and
that each selected file still exists. Music is only required when **Include
music** is enabled. Use **Check inputs** before queueing. If an output already
exists, choose another filename or move the existing result; queued jobs use
no-clobber output publication.

Open the job or history details for the error, structured diagnostics, and
performance record. CLI users can inspect the queue with:

```powershell
& $renderer jobs list
& $renderer jobs inspect 'job-id'
```

Engine ZIP, level, and resource inspection commands can examine inputs without
running a full export. Unsupported or malformed required data is reported as
an error; optional media is not silently substituted.

## Sono-GCC is unavailable

Sono-GCC is optional. The Sono VM remains the default and semantic reference.
Native compilation needs `rustc` from `RUSTC` or `PATH`; the release package
does not contain a compiler. If the compiler is missing or a native region
cannot be used, the existing policy keeps supported work on the VM. Security
software may flag generated native DLLs; do not disable it to use Sono-Renderer.

## Cache or project problems

- Cache corruption: use **Cache Management** to validate or remove the
  affected derived entry. A cache miss may make the next eligible render
  slower, but it does not change VM semantics.
- Missing project input: repair the exact engine, level, resource, or media
  reference in the project. The app does not select a replacement based on a
  filename guess.
- Project schema problem: keep a copy of the file and read the full validation
  message. Unsupported future schema versions are rejected rather than
  silently rewritten by an older app.
- Invalid output path: select a writable destination with sufficient free
  space. Sono-Renderer does not redirect a requested output to another folder.

## CLI and platform checks

For the installed package, define `$renderer` once in PowerShell:

```powershell
$renderer = Join-Path $env:LOCALAPPDATA 'Programs\Sono-Renderer\Sono-Renderer.exe'
& $renderer validate-config 'renderer-settings.json'
& $renderer ffmpeg-info
```

Use `& $renderer <command> --help` for the current arguments. User paths may
contain spaces and Unicode characters; quote each path in PowerShell. This
release package supports Windows x86_64. Linux and macOS have not been
packaged or tested.

See [Installation](INSTALLATION.md), [Getting Started](GETTING_STARTED.md),
[Chart Studio](CHART_STUDIO.md), and
[Gameplay presentation and preview](PRESENTATION_STUDIO.md) for the supported
workflows.
