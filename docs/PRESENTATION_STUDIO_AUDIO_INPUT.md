# Presentation Studio microphone recording

> **Availability:** Presentation Studio is hidden from the default desktop
> navigation. This guide describes retained editor infrastructure.

## Current implementation

Presentation Studio owns the microphone controls. It uses CPAL 0.18 for the
platform audio-input backend; capture does not initialize Sonolus, WatchVm,
Sono-GCC, Chart Studio, LevelData, or gameplay resources. Device enumeration
does not open a capture stream. Capture begins only after **Test Input / Meter**
or **Record Voiceover** is pressed.

The device picker offers the system default and enumerated input devices. The
chosen CPAL device identity is saved in
`presentation-audio-input.json` under the renderer's settings directory.
Display names are shown for people; selection uses CPAL's device ID where the
host provides one. If a saved device disappears, the UI reports that state.
Starting with that stale choice falls back to the current system default and
reports the fallback. While a stream is active, device enumeration runs every
two seconds; backend errors or disappearance stop capture and finalize the
audio received so far when possible.

**Test Input / Meter** opens the selected input only for level measurement and
discards its samples. It shows callback RMS and peak, latches a clipping
indicator, and displays input-capture-to-callback delay when CPAL reports it.
Monitoring through speakers is not implemented and defaults off. This avoids
creating feedback paths or silently listening when Presentation Studio opens.

Recording converts supported CPAL PCM/float formats into signed 16-bit PCM WAV
at the device's default sample rate and channel count. Mono stays mono in the
WAV. The file is staged in the destination directory as
`*.wav.partial`; the capture callback uses a bounded queue and never writes to
disk. A writer thread streams queued samples, checkpoints the WAV header and
syncs data about every two seconds, then flushes and validates the final header
before atomically renaming to `.wav`. RIFF/WAV's 4 GiB size limit ends a take
with a warning. If the writer queue overruns, capture stops accepting further
samples and preserves the last complete queued block rather than blocking the
audio callback.

For a saved project, takes go in `<project directory>/media/recordings/`.
Before a project is saved, takes go in the renderer's per-user `recordings/`
directory. The composition save path stores a relative media path when it can;
otherwise the existing composition path rules retain the absolute location.
No microphone data is added to diagnostics or uploaded. A finalized take is an
ordinary `MediaSource` with an ordinary `ClipKind::Audio` clip. It receives the
same bounded 512-bin peak summary used for imported audio, appears in the
existing media list, and goes through the existing FFmpeg composition mixer.
Captured WAV stays lossless while editing; composition export encodes delivery
audio as AAC. The source's native sample rate is retained until export.

Each staged take has a small versioned sidecar containing only timeline start,
sample rate, and channel count. When Presentation Studio opens, it scans the
project recording directory and the per-user recording directory for at most
256 interrupted takes in each location. It lists recoverable takes without
inserting them. **Recover and insert** repairs the WAV header from flushed
sample bytes, discards a partial trailing sample frame, promotes the file, and
adds it as ordinary project audio at its saved timeline position. A recovered
take is marked for boundary review. An abrupt power loss can still lose data
that the OS or device had not flushed.

Microphone preference saves use a synced staging file and atomic replacement.
The track edit stack stores up to 40 composition snapshots. Undoing a take's
timeline insertion preserves the media source and does not delete the WAV.

## Synchronization and supported workflow

The recorded clip starts at the playhead position captured when Record is
pressed. Sample position advances from input callback frames at the device's
actual sample rate. GUI repaint intervals do not determine take duration. When
the user chooses Record Voiceover, Presentation Studio starts continuous
timeline playback and then starts input capture; the playback clock is the
system audio output's submitted sample-frame count, while recording duration
is based on input callback frames. The capture backend's reported input latency
is shown for information only; the renderer does not add a magic offset.

Presentation Studio now has continuous video/audio playback, seek, scrub, and
frame-step, so the user can narrate while the composition plays. This
implementation has not been validated for physical input-to-output latency,
capture/playback drift over a long take, or recording in a packaged build.
Pre-roll/countdown, microphone monitoring output, recording pause, measured
clock compensation, and long-duration drift correction remain deferred.

Recorded WAVs use the same timeline and waveform path as imported audio. Audio
clips support move, trim, split, duplicate, delete, mute, solo, gain, pan,
fade-in/fade-out, and animated gain/pan keyframes. Audio tracks also have mute,
solo, and track gain. The inspector offers a crossfade action for eligible
overlapping clips on one track. The shared composition filter graph mixes
those clips for preview and export. Per-clip audio effect stacks such as EQ or
compression are not implemented. No microphone-specific timeline or mixer
model was added.

## Validation status

- **Build-checked:** Windows x86_64 `cargo check --locked` compiles CPAL's
  Windows backend and the GUI capture path. The build emits the existing
  warning that `rc.exe` is unavailable for executable version metadata.
- **Unit-tested:** PCM WAV writing, checkpoint recovery before a two-second
  checkpoint, trailing-frame repair, stable device preference serialization,
  shared project source/clip creation, waveform generation, project save/reopen,
  device enumeration safety, and shared mixer mute/solo/pan graph generation.
- **Enumerated:** the current test environment returned one CPAL input device.
  This does not establish that a physical microphone produced audio.
- **Not tested with a real device:** microphone permission denial, audible
  capture, unplug during capture, short/long recording, capture-to-playback
  latency/drift, clipping from a physical signal, or packaged-app capture.
- **Not interactively tested:** the Presentation Studio controls and actual
  voiceover export in the packaged application.
- **Playback integration implemented, physical synchronization unverified:**
  recording can start composition playback and insert an ordinary WAV clip at
  the captured playhead. This does not establish measured physical-device
  A/V alignment or long-run drift behavior.

The editor remains independent of Sonolus inputs when used as a general video
composition project. The capture module references only the shared storage
root and composition media boundary; it does not create a second project,
mixer, waveform, recording queue, or media-asset schema.
