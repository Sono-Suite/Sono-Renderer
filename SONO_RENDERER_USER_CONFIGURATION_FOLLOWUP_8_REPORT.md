# Sono-Renderer Follow-up #8: User Configuration

## Result

The desktop GUI and CLI now use the same serialized `RenderConfig`. Users can
save/load that configuration in the GUI or create, validate, and render it from
the CLI. Configuration values are validated before a config-driven export;
relative project and embedded profile-resource paths resolve beside the JSON
file.

The complete setting reference, defaults, ranges, and frontend availability is
in [docs/USER_CONFIGURATION.md](docs/USER_CONFIGURATION.md).

## Existing surface audit

The repository had no automatic desktop preferences file or separate user
configuration format. The desktop stored an in-memory `RenderConfig`, and the
CLI exposed settings through `render-video`, `run-watch`, and diagnostic
subcommands.

The pre-existing CLI commands and flags are retained. This includes positional
`render-video` inputs; timing, dimensions, FPS, backend, VM/GCC, worker,
profiling, layer, level-option, UI-layer, MV, SFX, Watch-trace, and display-list
options; and all existing inspect, compatibility, compile, and export
subcommands. A command inventory and notable retained options are in the
configuration reference.

The existing main-GUI **Project / inputs**, **Engine options**, and **Render /
video** tabs remain. Existing project pickers, named and arbitrary engine
options, start/duration/whole-chart controls, VM/GCC selection, scheduling,
dimensions/FPS, CPU/WGPU, MV, layer switches, determinism validation, progress,
logs, and cancellation remain available. GUI first-run dimensions, FPS,
duration, backend, and deferred GCC compilation behavior are preserved.

No existing setting was removed or renamed. There was no older standalone
settings-file format to migrate. New `RenderConfig` fields have serde defaults,
so older serialized configs retain their prior settings and receive the
existing renderer defaults for newly represented values.

## Added configuration

- Shared JSON config persistence through GUI **Load configuration...** and
  **Save configuration...** controls.
- CLI `default-config`, `validate-config`, and `render-config` commands.
- CLI `--ui-profile`, `--watermark-profile`, `--no-watermark`,
  `--moving-watermark`, and `--no-moving-watermark` options on `render-video`.
- GUI Gameplay UI controls for profile loading, enable/mode, and built-in HUD
  component visibility. Custom element layouts, expressions, assets, and fonts
  remain in generic profile JSON.
- GUI Watermark controls for the master switch, moving mark, text/appearance,
  anchors, movement, animation, timing, and layer. The fixed attribution text
  and bottom-left location are displayed as immutable.
- Shared H.264 `video_crf` (0..=51, default 18) and `video_preset` controls in
  config JSON, the GUI, and `render-video --video-crf/--video-preset`. Defaults
  preserve the existing medium preset and CRF 18.
- Watch worker count, profile diagnostics, and the existing supported renderer
  backend/execution/layer settings are persisted in the shared config.
- CLI `clear-gcc-cache` and a confirmed GUI cache-management action. Cache
  identity, storage location, and automatic cache use are unchanged.

CRF 22 with the `fast` preset is exercised through a real MP4 export test.
The attribution string is encoded in Rust Unicode escapes to protect the exact
required characters from source-encoding corruption; its test asserts the
required text.

## Invalid configuration and paths

`RenderConfig::validate` checks dimensions, FPS, start/duration, workers, level
options, MV brightness, gameplay UI, watermark, and CRF range. The config CLI
preflight additionally checks required project files, BGM requirements, even
MP4 dimensions, optional media/resource files, and profile asset decoding.
Missing/invalid enabled profile resources return contextual errors.

The GUI validates before starting export. Its selectable UI and watermark
profiles use the same `RenderConfig` APIs as CLI profile flags. Profile paths
are resolved at load and normalized for embedding; resources in a shared JSON
config resolve beside that file. No resource parsing occurs per frame.

## Compatibility and limitations

All new CLI operations are additive. JSON is the shared complete configuration
surface; existing one-off flags continue to work and new direct flags are added
for profile selection, watermark opt-outs, and encoder quality/preset.

The encoder currently supports MP4/H.264 and the existing audio pipeline; the
container/video codec and audio settings remain fixed. WGPU adapter/device
selection is not exposed. Sono-GCC cache location and enablement remain
automatic, though users may clear the cache. Custom gameplay UI element trees
remain profile data; this follow-up does not create a full profile editor.

## Validation

Validation performed:

- `cargo fmt --all -- --check`
- `cargo check --offline --all-targets`
- `cargo test --offline --all-targets` (141 library tests passed, 12 CLI tests
  passed, 3 `arctan2` tests passed, 101 `m0` integration tests passed, 9 Watch
  scheduler tests passed; 3 fixture-dependent tests ignored)
- Focused post-encoder tests for config validation, CLI parsing, and a real MP4
  export using non-default CRF/preset settings
- `git diff --check`

The full all-target suite was run before the final encoder-option wiring; the
final wiring was then checked with all targets and exercised by the focused
real-export test above.
