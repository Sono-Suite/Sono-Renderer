# Chart Studio tempo analysis

> **Availability:** Chart Studio is hidden from the default desktop navigation.
> This guide describes retained editor infrastructure.

## Workflow and ownership

In Chart Studio, choose a local audio source or use the current project's
Music/BGM input, then select **Analyze Tempo**. URL-acquired audio remains an
ordinary Music/BGM asset; after **Download and assign**, choose **Use current
project Music/BGM**. Analysis runs on a background worker and reports progress,
can be cancelled, and is discarded if the selected path/generation changes.
The analysis and waveform paths require the application's already-installed
FFmpeg/FFprobe pair. They do not download tools or contact a BPM service.
FFmpeg protocol access is restricted to local/file and pipe-style protocols
for analysis decoding.

The analyzed media is decoded by the same FFmpeg installation used by
composition playback. Analysis downmixes and resamples that decoded stream to
mono 11 kHz, then retains only a 220 Hz positive-RMS-flux envelope; it does not
retain full-song PCM. The resulting beat positions are audio media seconds.
MP3 demux/decode honors the same FFmpeg gapless metadata path as playback. A
conditional integration test encodes a known click track to MP3, compares the
analysis decode with the production composition playback filter graph, and
checks near-start, midpoint, and near-end beat alignment. The test reports a
skip when the application's discovered FFmpeg pair or its MP3 encoder is unavailable.

## Proposal quality and limits

The detector searches normalized onset-envelope autocorrelation for a global
period, keeps half/double interpretations where in range, estimates reference
phase from the first strong onset, and generates evenly spaced candidate beat
markers. Beat markers are a projected grid, not independent per-beat tracking.
The displayed score and High/Medium/Low quality are explicitly heuristic, not
probabilities. Empty/quiet or short sources return no reliable tempo. Local
15-second windows can flag that tempo appears to vary; this is only a warning,
not a tempo map. No downbeat or meter inference is performed.

The selected candidate updates the waveform overlay without rerunning
analysis. Numeric BPM and phase fields allow correction; 10 ms phase nudges
are available. Tap Tempo uses recent tap intervals and a median/MAD outlier
filter. Playback uses the ordinary composition audio timeline and output clock;
it does not initialize Watch. The schema-declared entity list can show each
entity's nearest reference beat and signed offset. `WatchSeconds` fields are
converted to media time through the existing
`media_time = Watch_time + bgmOffset` rule before comparison. This is an
annotation only: it does not enable automatic entity snapping or change the
chart timeline's engine-defined semantics.

**Apply reference grid** is explicit and undoable. It changes only the
ChartDocument's authoring metadata. Existing entity times, `bgmOffset`, and
engine-defined `#BPM_CHANGE`/`#TIMESCALE_CHANGE` entities are preserved. The
grid is saved under the existing `chartStudioLevelData` extension in a
`.sonoproject`, and is restored on reopen. Standalone `level.json.gz` is the
engine-native LevelData format and intentionally does not include this
renderer-only reference grid. Clearing the analysis cache never deletes an
applied grid.

## Cache and bounded work

The cache is keyed by source SHA-256, detector version, decode/envelope
settings, accepted BPM bounds, and the rhythm score floor. It stores compact,
versioned JSON candidates only, under the existing application cache root. It
retains at most 256 entries and prunes old entries in small batches. Cache
inventory reports its count/size, and the existing cache clear action removes
its owned records. Cache content is disposable; project grid metadata is the
accepted authoring state.

The estimator caps decoded input at four hours and bounds its quadratic lag
search for longer files by evaluating four evenly spaced 30-second windows.
Shorter files use the complete envelope. Full decoded PCM is never retained.
Stage durations are exposed in the result panel as decode, estimate, and cache
lookup milliseconds; they are diagnostic observations, not performance
regression assertions.

## Library evaluation

The mature C-based aubio tempo/onset APIs were evaluated, but aubio and its
Rust sys binding are GPL-licensed and introduce native build/package and FFI
requirements into this cross-platform desktop release. The Rust `stratum-dsp`
package offers broader beat tracking under permissive licensing, but its
published package is recent and its benchmark claims are self-reported; it
also brings a significantly larger DSP/decode stack than this narrow local
proposal needs. `beat_track_rs` is an older, low-maturity implementation, and
RustFFT is a transform primitive rather than a beat tracker. The chosen
detector is therefore a small, deterministic, inspectable autocorrelation
proposal built over the renderer's existing FFmpeg decode path. It is not
represented as equivalent to a mature music-information-retrieval system.

References: [aubio documentation](https://aubio.org/documentation),
[aubio-sys license manifest](https://docs.rs/crate/aubio-sys/latest/source/Cargo.toml.orig),
[stratum-dsp package documentation](https://docs.rs/crate/stratum-dsp/latest),
[beat_track_rs documentation](https://docs.rs/beat-track-rs/latest/beat_track_rs/),
[RustFFT package documentation](https://docs.rs/crate/rustfft/5.1.0).

## Validation status

Deterministic synthetic tests cover 60, 90, 120, 128, 150, 174, and 180 BPM,
fractional BPM 128.5 and 174.25, known phase, 87/174 half/double candidates,
silence, short input, random noise, a five-second silence intro, a 120-to-160
tempo change warning, Tap Tempo outlier handling, and cache read/write/clear
isolation. The MP3 test is a real FFmpeg integration path when the
application-discovered tool pair is present. The ChartDocument test protects undo/redo and confirms
that LevelData remains byte-semantically unchanged; a RenderProject save/reopen
test covers persistence. Synthetic signals do not establish accuracy on
arbitrary real music. Downbeat, meter, variable-tempo-map, automatic snapping,
auditory human review, and packaged-install interactive validation remain
outside this implementation's verified claims.
