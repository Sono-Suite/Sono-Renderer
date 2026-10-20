# Sono-Renderer Follow-up #34 report

## Result

Chart Studio now offers a local tempo proposal, half/double candidates,
waveform grid overlay, song audition, manual BPM/phase editing, Tap Tempo, and
an explicit undoable **Apply reference grid** action. Analysis output remains a
proposal. Applying it stores renderer-only chart authoring metadata and does
not change LevelData entity timestamps, `bgmOffset`, or engine-defined
`#BPM_CHANGE`/`#TIMESCALE_CHANGE` entities.

## Existing architecture and ownership

- Composition media probing, waveform generation, and playback already use
  FFmpeg. Timeline playback decodes through FFmpeg and uses the audio-device
  sample clock; the GUI repaint cadence is not the audio clock.
- Chart Studio authors canonical `formats::LevelData`. Before this work it had
  no audio playback/waveform, universal BPM grid, beat-snapping command, or
  engine-independent BPM map. Watch retains ownership of engine-specific beat
  conversion and BPM behavior.
- The applied reference grid is `ChartBeatGrid` inside `ChartDocument`, with a
  single undo operation. It is serialized in the existing
  `chartStudioLevelData` render-project extension. Standalone engine-native
  `level.json.gz` intentionally omits renderer-only grid metadata.
- Analysis uses existing FFmpeg/FFprobe binaries only. It checks installed
  tools without provisioning them and limits FFmpeg/FFprobe protocols to local
  file/pipe-style access. The decoder streams mono 11 kHz float PCM into a
  220 Hz onset envelope; full-song PCM is not retained.
- The source identity uses SHA-256. Cache identity additionally includes the
  detector version and relevant decode/envelope/BPM/score settings. The small
  versioned JSON cache is under the existing app cache root, capped at 256
  entries, and is visible to the existing cache inventory, clear, and prune
  workflows.

## Detector and review semantics

The detector is a deterministic positive-RMS-flux/autocorrelation proposal.
It reports a primary global tempo and in-range alternate metrical
interpretations, estimates phase from the first strong onset, and projects
beat timestamps from that phase and period. Those timestamps are a proposed
grid, not independently detected per-beat events. The score and High/Medium/
Low quality labels are heuristic, not probabilities. Silence and unsuitable
short audio return “No reliable tempo.” Local 15-second windows may report
that the track appears to vary; they do not create a tempo map. Downbeat and
meter detection are not implemented.

Candidate selection updates the grid without another analysis run. Users can
change BPM, edit phase, nudge phase by 10 ms, audition audio, and use Tap Tempo.
Tap Tempo changes BPM only; it does not invent a phase. The waveform and
schema-declared Chart Studio timing list show grid alignment. Watch-second
fields are compared after conversion to audio media time using
`media_time = Watch_time + bgmOffset`; this is annotation only, not automatic
snapping.

## Library review

- aubio provides mature C tempo/onset functionality, but its GPL licensing and
  native build/FFI packaging requirements were judged a poor fit for this
  cross-platform release. Its Rust system binding is also GPL-licensed.
- `stratum-dsp` has a permissive license and broader beat-tracking features,
  but its published package is recent, its benchmark figures are package
  claims rather than independently reproduced results, and its added DSP/decode
  stack is much larger than this local proposal needs.
- `beat_track_rs` is a lower-maturity older implementation. RustFFT is a
  transform primitive, not a complete beat tracker.
- The selected detector is consequently small and inspectable, and its limits
  are shown in the UI/docs. It is not claimed to match a mature music
  information retrieval system.

References: [aubio documentation](https://aubio.org/documentation),
[aubio-sys license manifest](https://docs.rs/crate/aubio-sys/latest/source/Cargo.toml.orig),
[stratum-dsp documentation](https://docs.rs/crate/stratum-dsp/latest),
[beat_track_rs documentation](https://docs.rs/beat-track-rs/latest/beat_track_rs/),
[RustFFT documentation](https://docs.rs/crate/rustfft/5.1.0).

## Validation evidence

- **Build-checked:** `cargo fmt -- --check` and `cargo check --all-targets`
  passed. The build emitted the repository's existing `rc.exe unavailable`
  warning; executable version metadata was not embedded.
- **Unit/synthetic-audio tested:** tempo values 60, 90, 120, 128, 150, 174,
  180, 128.5, and 174.25 BPM with known phases; 87/174 alternatives;
  silence/short/random-noise behavior; five-second silence intro; 120-to-160
  tempo-change warning; Tap Tempo outlier handling; three-minute global-window
  path; cache serialization/corrupt-record replacement/cleanup isolation.
- **MP3 integration tested:** this environment had an application-discovered FFmpeg pair
  and MP3 encoder. A generated 120 BPM PCM click was encoded to MP3, decoded by
  the analysis path, and decoded again by the production composition playback
  filter graph at the same comparison rate. Both estimates passed the 1 BPM
  bound; phase differed by less than 25 ms; predicted beat positions at about
  0.137 s, 25.137 s, and 49.137 s passed 50 ms bounds and analysis/playback
  comparison passed 25 ms at those points. This validates the synthetic MP3
  timebase path, not arbitrary encoders or real songs.
- **Chart model/integration tested:** ChartDocument apply/undo/redo preserves
  the exact previous LevelData value and `bgmOffset`. RenderProject save/reopen
  restores the accepted grid. The existing authoritative Watch-export fixture
  also passed in the full suite; this does not mean the reference grid changes
  Watch semantics.
- **Full library regression tested:** `cargo test --lib -- --test-threads=1`
  finished with **381 passed, 0 failed, 6 ignored** in 804.68 seconds. The six
  ignored tests require private fixtures or optional integration conditions.

## Not verified / intentionally deferred

- No independent copyrighted real-music corpus was used; detection quality on
  ordinary songs remains for human listening and broader labeled evaluation.
- No actual yt-dlp URL was acquired in this run. The existing acquisition UI
  assigns audio to ordinary Music/BGM, and Chart Studio exposes **Use current
  project Music/BGM**; that handoff was verified by code inspection, not a live
  network integration run.
- No interactive GUI session or installed packaged application was exercised.
  The UI is build-checked and its model commands are tested, but visual layout,
  device-specific playback, and packaged dependency discovery still need
  manual platform validation.
- This does not provide a tempo map, per-beat transient tracking, downbeats,
  meter inference, automatic entity retiming/snapping, or BPM_CHANGE
  generation. Those require engine-specific authoring semantics and explicit
  user review.
- Stage durations (source identity hash, decode, estimate, cache lookup) are
  recorded and displayed. A representative hardware benchmark report was not
  collected in this pass.
