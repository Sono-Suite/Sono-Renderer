# Sono-Renderer Follow-up #35 — Automatic Subtitles

## Result

Presentation Studio now has a local transcription path, timed subtitle interchange, editable caption clips, and normal compositor burn-in. The implementation extends its existing composition/text model; it does not introduce a second timeline or subtitle renderer.

## Architecture and workflow

- The source of truth remains ordinary `CompositionClip` fields: `timeline_start`, `duration`, and `text`. Optional `CaptionMetadata` stores provenance only. `SubtitleCue` is an import/export/transcription interchange value, not another project timeline.
- A user selects one audio-bearing audio/video timeline clip. The backend transcribes only that media source's first audio stream, not every audible track or the full composition mix.
- FFmpeg extracts the selected source window, applies the clip's forward playback rate, resamples to 16 kHz mono PCM, and limits output to the clip's timeline duration. Recognition cue times are clip-local; insertion adds the clip's composition start and clips cues to the selected interval.
- Imported audio, video audio, recorded WAVs, and acquired media participate through their existing media-source model. EngineData, LevelData, WatchVm, Sono-GCC, and Chart Studio are not dependencies of transcription or caption editing.
- The selected project title preset creates ordinary text clips. Captions use the existing text editor, visual/text controls, timeline, undo history, preview, save, and MP4 compositor/export path. The built-in short-form preset is data in the existing title-preset list.

## Backend and dependency handling

The selected recognizer is the `whisper.cpp` command-line runtime with its multilingual `ggml-base` model. This optional process boundary avoids Python, a new Rust FFI/build dependency, and a CUDA requirement while allowing existing job control to cancel/reap inference. Cloud transcription was excluded to keep audio local. The application pins the Windows x64 CPU runtime at v1.9.2 and verifies the downloaded runtime ZIP and model with SHA-256 before installation/use. The runtime is about 8 MB compressed in the pinned upstream release; the model is 147,951,465 bytes (about 148 MB). The runtime is MIT licensed; local license/attribution notices are installed beside the runtime/model. See the [whisper.cpp upstream project](https://github.com/ggml-org/whisper.cpp), its [release list](https://github.com/ggml-org/whisper.cpp/releases), and the [multilingual Base model card](https://huggingface.co/ggml-org/whisper.cpp).

Downloads are separate, explicit user actions. They use HTTPS, size limits, pinned hashes, staged installation, and progress reporting. After installation, recognition works without network access. Audio is not uploaded. CPU inference is supported and does not require CUDA. Managed runtime/model installation and the integrated setup controls are Windows-only; no external-recognizer path or non-Windows installer is currently exposed. Model/runtime files are not embedded in the renderer installer.

## Timed text and editing capabilities

| Area | Current behavior |
|---|---|
| Speech sources | One selected clip with audio; includes ordinary audio/video media. Composition mix and nested-composition sources are not supported. |
| Source time | Source In and forward constant playback speed are applied. Reverse/freeze is rejected. Composition offset and range clipping are applied at insertion. |
| Recognition output | Whisper phrase/segment times and text. Word timing and calibrated confidence are not synthesized. Speaker diarization is not implemented. |
| Segmentation | Uses recognizer-generated segments; there is no separate user-configurable segmentation engine. Human review is expected. |
| Languages | `auto` or a two/three-letter Whisper language code. Multilingual Base is used; automatic detection and mixed-language recognition are not guaranteed. |
| Editing | Edit text in the normal text inspector; move/trim like other clips; split at playhead; merge two selected compatible captions. Insertion, split/merge, and styling use existing project undo behavior. |
| Styling | Existing title presets and text rendering controls own font, position, size, color, outline, shadow, background, effects, transforms, and animation. Short-form captions are an ordinary reusable project preset. |
| Word emphasis | Deferred because this backend path does not emit word timestamps. No guessed karaoke timing is saved. |
| Import/export | SRT and WebVTT are parsed into the same cue model and ordinary clips. Export emits SRT or WebVTT in composition time. VTT markup/settings, cue IDs, and speaker tags are normalized away; text/timing/Unicode remain. |
| Video output | Captions burn in through normal Presentation Studio text/compositor/export. Embedded MP4 subtitle tracks are not implemented. |

SRT/VTT parsing is UTF-8, size/cue-count/text bounded, checks timestamp ordering and range, and rejects malformed timing. The project schema is now 21. Schema 20 migration adds the short-form preset without removing user presets; caption provenance is an optional field defaulting to absent for old text clips. Existing project text/timing stays authoritative.

## Jobs, cache, cancellation, and failure behavior

Transcription and dependency installation run through Presentation Studio's existing single background-job slot. The UI is not blocked. Dependency downloads report byte progress; inference reports task stages and remains indeterminate while the CLI runs. Cancellation uses the shared child-process control, reaps the process, and inserts no partial captions. A composition digest is captured before inference; if the composition changed, the result is rejected as stale.

Decoded selected audio, runtime/model identity, language, and recognition settings participate in the transcription-cache identity. Raw transcript JSON is disposable and bounded to 512 records / 64 MiB. Clearing it does not remove accepted/edited project captions. Setup paths reject symlinks for the local runtime/model and transcription cache writes. Incomplete subtitle imports, missing media, bad model/runtime hashes, unavailable FFmpeg, and unsupported source-time modes return an error rather than inserting partial cues.

Only one Presentation Studio job runs at once, and the recognizer uses at most eight CPU threads. Inference performance was not measured because the optional runtime/model were not installed; no throughput or quality promise is made.

## Validation evidence

| Claim | Evidence |
|---|---|
| Rust formatting | `cargo fmt --all -- --check` passed. |
| Build | `cargo check --locked --all-targets` passed on the current Windows target. The build emitted the environment warning that `rc.exe` is unavailable, so executable version metadata was not embedded. |
| Subtitle unit tests | `cargo test --locked --lib subtitles::tests`: 5 passed, 1 ignored. Covers SRT/WebVTT, Unicode, malformed/oversized timestamps, ordinary text-clip insertion/persistence, and split/merge. |
| Composition tests | `cargo test --locked --lib composition::tests`: 61 passed, 1 ignored, including schema migrations and preservation of user presets. |
| Full serial library suite | `cargo test --locked -- --test-threads=1` ran 394 tests: 382 passed, 7 ignored, and 5 initially failed only because old assertions hard-coded schema version 20. Those assertions were updated to 21 and the composition group then passed. The full suite was not rerun after that assertion-only correction. |
| Clippy | Strict `cargo clippy --locked --all-targets -- -D warnings` is not clean: repository-wide lints stop the run (187 diagnostics across existing renderer/audio/UI code). The focused subtitle module's two Clippy findings were corrected; a subsequent filtered Clippy run reported no `src/subtitles.rs` diagnostics. |
| FFmpeg extraction | An opt-in real-media test was added for trim, 2x retiming, 16 kHz mono PCM, and resulting duration. It was not run because this machine has no validated FFmpeg/FFprobe installation. |
| Real speech | Not tested. The whisper runtime and 148 MB model were not downloaded; no real-speech accuracy, silence/music hallucination, or multilingual result is claimed. |
| GUI/package | Build-checked only. No interactive desktop workflow or packaged release was exercised. |

## Remaining limitations

- Real-world transcription accuracy, languages, silence/music behavior, and model latency need testing with the optional runtime/model and representative speech.
- FFmpeg-backed decoding is required. The local environment lacked FFmpeg/FFprobe, so source extraction and final burn-in were not exercised against real media here.
- Non-Windows dependency provisioning, GPU inference, word timestamps/highlighting, speaker diarization, configurable segmentation, mixed-language diarization, nested-composition transcription, reverse/freeze source mapping, composition-mix transcription, and embedded subtitle streams are not implemented.
- The built-in caption styles are starting presets, not a professional caption-template browser. Users can customize/save them through the existing title-preset system.
- The transcription progress display is indeterminate during inference because the current subprocess capture path does not stream Whisper's internal progress output.

## Acceptance status

The implementation provides the offline standalone NLE workflow structurally: select a clip, transcribe locally, review/edit timed text clips, style them through Presentation Studio, and export them through the normal compositor. Build and deterministic model tests pass. End-to-end acceptance with real speech, preview frames, and an exported MP4 remains **unverified** until the optional model and FFmpeg are available and the desktop workflow is exercised.

## Follow-up #36 validation addendum — 2026-10-08

The earlier report's statement that FFmpeg/FFprobe were unavailable describes the #35 validation environment at that time. During Follow-up #36, a matching FFmpeg/FFprobe 8.1 pair was found in the repository's shared local add-on/dependency area and validated by version output. The ignored production integration tests for standalone NLE preview/export, decode-pipe lifetime, trimmed/rate-adjusted subtitle audio extraction, and project → queue → real render were run and passed. This updates the FFmpeg evidence only: the Whisper runtime/model remained unavailable, so no real speech inference, subtitle burn-in workflow, or packaged inference test is claimed. See [the #36 feature-existence audit](FEATURE_EXISTENCE_AUDIT.md) for exact commands and boundaries.
