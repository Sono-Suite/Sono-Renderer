# Sono-Renderer Follow-up #6 — Gameplay UI

## Scope

Follow-up #6 adds renderer-owned, profile-driven gameplay UI configuration, layout, rendering, and an explicit external gameplay-event adapter. Profile loading remains at the renderer/configuration API boundary. This work does not add a desktop GUI profile picker/editor, public CLI profile flags, or a temporary user-facing editor.

The UI compositor is a CPU post-composite pass over normal CPU or WGPU scene output. It does not replay Watch or derive score/judgement from chart entities and scheduled effects. `legacy` and `off` modes remain available. Engine-authored UI draws remain part of the rendered scene; the current interface cannot selectively replace one engine HUD draw with a profile image.

## Reference investigation

### Screenshot and Sono-Overlay source

I used the supplied screenshot and `TestingSuite/Sono-Overlay` as composition references. The source paths inspected were:

- `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2`
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/main2_16-9_1920x1080.object`
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/main2_4-3_1440x1080.object`
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/exo-alias.go`

The source combines score background, clipped score bar, grade markers, grade-specific rank glyph and caption, then score digits/delta; life background with normal/danger/overflow fills; combo glow/title plus per-digit foreground and temporary burst; and judgment sprites. The two layout templates are authored separately around canvas center. They place score near upper-left, life and pause upper-right, combo right of the field, and judgement around the lower-center of the gameplay image. The custom profile expresses those relationships in its own top-left normalized canvas with image, gauge, bitmap-font, anchor, z-order, visibility, and event-time animation features.

`sekai.obj2` also supplies useful transient timing: judgement presentation lasts 20 frames at 30fps (about 0.667s), the combo number scales in over 8 frames (about 0.267s), and the special burst fades/scales over 14 frames (about 0.467s). These are represented as generic profile animation durations anchored to externally supplied event timestamps.

The reference movie confirms that the supplied target image corresponds to the `Larp 64x` presentation. A full-resolution frame at 24.800s displays score `00489874`, rank B, life 1000, combo 41, and PERFECT, matching the supplied screenshot's HUD state. At 25.100s the movie has advanced to score `00517813` and combo 42. The validation fixture is placed at 24.833s, the nearest 12fps Watch frame to that matching 24.8s source moment.

### Video behavior review

All five clips in `TestingSuite/ui_ref` were checked with their metadata and representative time samples. For the Larp clip, I also extracted and visually inspected full-resolution frames at 24.800, 24.900, 25.000, 25.100, and 25.500 seconds, plus a one-second event window from 23s through 30s.

| Video | Metadata | Representative samples | Observed behavior used in validation |
|---|---|---|---|
| `Larp 64x Preview.mp4` | 30fps, 1920x1080, about 49.4s | 1, 10, 20, 23–30, 40, 48s; additional full-resolution event frames above | B score rank and score progression; combo steps 33→66 through the sampled window; transient PERFECT judgments appear and expire; life/pause stay static. The 24.8s frame matches the supplied HUD state. |
| `Teto Mashup Preview.mp4` | 60fps, 1920x1080, about 33.95s | 1, 7, 14, 21, 28, 33s | Score delta, nonzero combo, PERFECT, and full life are visible together. |
| `Baumkuchen x retry now preview.mp4` | 30fps, 1920x1080, about 151.8s | 1, 30, 60, 90, 120, 150s | Rank changes across C/B/A and combo grows into the hundreds. |
| `Cubism Preview.mp4` | 30fps, 1920x1080, about 99.87s | 1, 20, 40, 60, 80, 98s | High combo and S rank state. |
| `La Piece Chart.mp4` | 60fps, 1920x1080, about 66.69s | 1, 13, 26, 40, 53, 65s | Life reaches zero; the end screen shows All Perfect. That end screen is outside the in-game HUD profile. |

The video files are behavioral/presentation references, not inputs to Watch state. Observed score, combo, life, rank, and judgement are not imported as authoritative game results.

## Asset use

I inventoried the repository's `assets/` tree: 199 files total (195 PNGs and four MP4 clips). `ui-profiles/ASSET_USAGE.md` maps the relevant files, intent, composition, and reasons for not using nearby alternatives. In `ui-profiles/default.json`, the HUD now uses:

- `score/bg.png`, `score/bar.png`, and `score/fg.png` for the frame, clipped fill, and rank thresholds;
- score digits and shadows from `score/digit/`, and the actual English rank glyph/caption PNGs under `score/rank/chr/` and `score/rank/txt/en/`;
- `life/bg.png`, normal/danger/overflow fill art and mask, and life digit/shadow sprites;
- `combo/pe.png`, `combo/pt.png`, the `p` digit family, and a transient `b` burst family;
- the six `judge/*.png` judgment sprites.

The profile uses the actual artwork in separate elements so the renderer can preserve proportions while controlling layout, clipping, and event-driven state. It does not flatten the HUD into a single fixed-resolution image. It does not hardcode Project SEKAI or Sono-Overlay asset names or behavior in generic renderer code.

`autolive-en.png` is relevant, but the Watch frame already draws an AUTO control. The renderer currently offers no generic way to replace only that existing engine-authored draw; drawing the asset on top would create a second, offset button. The validation image therefore leaves the existing control visible. It is noticeably dimmer and a few percent left of the video reference. The fixed-value flattened life images, static full-canvas combo image, Japanese rank caption, alternate digit palettes, and unrelated menu/chart art are also intentionally omitted; details are in the asset map.

## Visual-validation artifacts

The development-only harness is `examples/ui_validation.rs`. It loads the real `Larp 64x` level, Next RUSH engine, and ProSeka Faithful 0.8.4 resources; renders Watch frames through the CPU renderer; and runs the production UI compositor over those pixels. It is not installed as a CLI feature or exposed through a temporary GUI. Reproduce the image set from the repository root:

```powershell
cargo run --offline --example ui_validation
```

The harness produces a bounded 12-PNG set under `artifacts/ui-validation/`:

| Artifact | Purpose |
|---|---|
| `default_assets_normal_16_9_1280x720_t24.833.png` | Normal profile initial state. No player event or rank is invented. |
| `reference_assets_special_hit_16_9_1280x720_t24.833.png` | Asset-backed reference state at 1280x720. |
| `reference_assets_special_hit_16_9_1920x1080_t24.833.png` | Same state at 1920x1080. |
| `reference_assets_special_hit_4_3_1024x768_t24.833.png` | Same state at a materially different 4:3 ratio. |
| `reference_before_special_t24.667_f296_16_9_1280x720.png` | Before the synthetic event. |
| `reference_judgement_rise_t24.750_f297_16_9_1280x720.png` | Early judgement animation, 50ms after event. |
| `reference_judgement_settled_t24.833_f298_16_9_1280x720.png` | Settled judgement and matching reference HUD state. |
| `reference_judgement_expired_t25.417_f305_16_9_1280x720.png` | Judgement after its visibility interval. |
| `miss_assets_16_9_1280x720_t24.833.png` | Synthetic miss, combo reset, and reduced life. |
| `minimal_animation_phase_0.667_bitmap_font_16_9_t24.833.png` | Minimal profile with a PNG glyph and nonzero animation phase. |
| `terminal_field_monitor_4_3_1024x768_t24.833.png` | Contrasting 4:3 profile. |
| `legacy_comparison_16_9_1280x720_t24.833.png` | Legacy-mode rendering comparison. |

The main scene frame is Watch global frame 298 at 12fps (24.833s), with 1280x720, 1920x1080, and 1024x768 renders for the profile comparison. The event window uses Watch global frames 296, 297, 298, and 305. The harness selects engine defaults `Nexint-V4-BG-min`, `ProSekaOfficial`, and `NexintWaterMark`, and enables particle rendering. UI progress duration is 20s. `artifacts/ui-validation/MANIFEST.md` records exact fixture paths, profile/state, timestamps, dimensions, selected scene resources, synthetic event inputs and reduced values, Watch counts, and CPU compositor timings. `SHA256SUMS.txt` lists the final PNG hashes.

The synthetic fixture contains 40 normal hit events and one special hit at 24.700s, with a fixture-only score mapping chosen to show score 489874, combo 41, life 1000, rank code B, bonus 931, and PERFECT at 24.833s. The miss PNG uses a separate synthetic miss event. These external adapter inputs demonstrate presentation behavior only; they are not claimed to be authoritative outcomes from Watch or the reference recording.

## Validation results

### Structural/programmatic checks

- `cargo run --offline --example ui_validation` — passed; generated the final 12 PNGs and `MANIFEST.md` from the Larp 64x Watch fixture.
- `cargo fmt --all -- --check` — passed.
- `cargo check --offline --all-targets` — passed.
- `cargo test --offline render_ui::tests -- --nocapture` — passed, 14/14 focused UI tests.
- `cargo test --offline` — 125 passed, 3 ignored, 1 failed. The failure is `video_export::tests::bounded_pipeline_matches_sequential_hashes_and_preserves_fifo`.
- The failed pipeline test was rerun alone with `cargo test --offline --lib video_export::tests::bounded_pipeline_matches_sequential_hashes_and_preserves_fifo -- --exact --nocapture` and failed again. Its simulated worker panics at `src/video_export.rs:1957`; the test then unwraps `None` at `src/video_export.rs:2899`. This is outside the UI files changed here and does not affect the generated validation frames, but the complete suite is not green.
- All five included profile/event JSON files parsed successfully. The validation directory contains 12 final PNGs with SHA-256 entries in `SHA256SUMS.txt`.

### What I visually inspected

I inspected the full-resolution Larp reference at 24.800s and the generated asset-backed render set at landscape, 1080p, and 4:3 sizes. I also inspected the event-window renders, which show the judgement hidden before the synthetic event, partially faded during its rise, visible after settling, and hidden after expiry. The rank panel, SCORE gauge artwork and marker layer, life art/pause, combo title/digits, bonus digits, and judgement sprite are visible with no obvious off-canvas clipping in those inspected captures. The main HUD locations and scale closely follow the source composition. The default background has a bright cyan shape behind the right-side combo, but the combo remains readable.

### Remaining visual mismatch and limits

The screenshots are real renderer frames with the engine's purple `Nexint-V4-BG-min` background, lane, and particles. They do not contain the yellow character/music-video image, watermark, or moving notes seen in `Larp 64x Preview.mp4`; the level fixture supplied to Watch does not provide that movie layer. This visible difference means the complete reference scene has not been reproduced. The AUTO control also remains the Watch-authored version; it is darker and slightly left of the reference asset. The user-provided screenshot and source video remain the visual authority for those scene layers.

No human-device/client validation was performed. The timestamped event states are synthetic and do not validate gameplay scoring. The current PNGs validate profile asset composition and the renderer's image/layout/event-time path over actual Watch output; they do not establish pixel equivalence with the complete client video frame.
