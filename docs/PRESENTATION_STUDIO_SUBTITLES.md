# Presentation Studio subtitles and captions

Presentation Studio can import subtitle files or transcribe one selected audio-bearing timeline clip with the optional local `whisper.cpp` CPU runtime. Transcription runs in a background task; it does not initialize Sonolus, Watch, Chart Studio, Sono-GCC, or a cloud service. Audio stays on this device.

## Generate and review

1. Open a Presentation Studio composition and select an audio or video timeline clip that contains audio. A video source with both streams is supported; the editor transcribes only that selected clip's audio stream, never the composition mix.
2. In **Subtitles / captions**, enter `auto` or a Whisper language code and choose a project-local title preset. Select **Generate subtitles from selected clip**.
3. If needed, install the optional CPU recognizer and download the multilingual Base model using the explicit buttons. They are separate actions; the model is about 148 MB and is not bundled into every installer. After installation they can be used offline.
4. Review the new text clips on the ordinary timeline. Edit their text in the existing text inspector, trim or move them as regular clips, split a caption at the playhead, or select two captions and merge them. Generated insertion is one undoable project edit. Captions are project data, so clearing the disposable recognition cache or uninstalling the model does not remove them.
5. Use the existing text presets and controls for font, position, size, fill, outline, shadow, background, effects, transforms, and animation. Captions render through the normal composition text/compositor path and are burned into the exported video.

For import, choose **Import SRT / WebVTT**. Imported cues are placed at the current playhead. Export uses **Export SRT** or **Export WebVTT** and writes caption clips in composition time. Cue text and Unicode are preserved. WebVTT cue settings/markup are normalized to plain text; styling, cue IDs, and speaker tags are not retained. There is no embedded subtitle stream in MP4 output.

## Source time and cache

The selected clip's source in-point and forward playback rate are applied while preparing the recognition input. The trimmed source audio is decoded/resampled to 16 kHz mono PCM and its speed is mapped to the clip's timeline duration. Recognition cue times are then offset by the clip's composition start and clipped to its timeline interval. Reverse/freeze clips and nested-composition sources are currently rejected because their speech-time mapping is not implemented. To transcribe narration without music, select the narration clip rather than a music clip or mixed composition output.

Raw local results are cached by decoded selected-audio content, model/runtime identity, language, and recognition settings. The bounded cache holds at most 512 JSON records and 64 MiB; it is disposable. Edited captions are stored in the `.sonocomp` document, separately from this cache. If the composition changes while transcription is running, the stale result is discarded rather than inserted at outdated times. Only one Presentation Studio background job runs at a time; the task can be canceled.

## Local dependencies and platform status

The Windows managed setup pins `whisper.cpp` v1.9.2 CPU runtime and its multilingual `ggml-base` model. The runtime ZIP and model bytes are checked against pinned SHA-256 digests before use. Downloads happen only after the user requests installation, use HTTPS, and are staged before being installed. The runtime MIT notice and model attribution notice are placed beside the installed files. Runtime and model live in the per-user application data directory, outside the project; generated cue data is self-contained in the project.

Current managed provisioning and the integrated UI are Windows-only. The code can use platform FFmpeg resolution for audio extraction, but it does not currently provide a non-Windows whisper runtime/model installer or a settings UI for selecting an externally installed recognizer. A missing runtime/model produces an actionable error; normal editing and export continue to work. CPU inference is supported without CUDA. The Base model is multilingual, but recognition quality depends on speech, language, recording conditions, and the selected language; automatic detection is not guaranteed.

The backend currently returns phrase/segment timestamps only. Word-level timing, confidence, speaker diarization, editable word highlighting, mixed-language speaker separation, and composition-mix transcription are not claimed or synthesized. `-sns` suppresses non-speech tokens, but it is not a voice-activity detector; silence/music may still produce recognition errors and results require human review.

## Validation

Focused deterministic model/interchange tests:

```powershell
cargo test --locked --lib subtitles::tests
cargo test --locked --lib composition::tests::schema_twenty_migration_preserves_user_presets_and_adds_short_form_style
```

The real FFmpeg selected-clip extraction test is opt-in and requires the already-installed, validated FFmpeg/FFprobe pair. It checks trim, 2x speed mapping, 16 kHz mono PCM, and resulting duration without downloading dependencies:

```powershell
cargo test --locked --lib subtitles::tests::real_ffmpeg_transcription_extraction_applies_trim_speed_and_mono_pcm -- --ignored --nocapture
```

Real speech recognition, multilingual accuracy, device-specific inference performance, and packaged-application execution must be validated on a machine with the optional runtime/model installed. No transcript is treated as ground truth; compare and edit it against the source speech.
