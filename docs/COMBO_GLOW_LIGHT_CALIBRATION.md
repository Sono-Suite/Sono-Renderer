# Combo Glow/Light calibration

## Scope and result

Calibrated the existing profile-backed combo raster effects against the supplied
`TestingSuite/DisplayHolic Blank.mp4`. The production HUD compositor is used for
all generated samples. No scoring, Watch VM, glyph assets, digit spacing,
transforms, or source animation curves were changed in this calibration.

The late-pop halo and brightness response are substantially closer across eight
combo values. **This is an empirical filter calibration, not exact AviUtl pixel
parity.** There are still rasterization, edge-color and filter-shape differences.

## Reference identity and timing

FFprobe measured:

| Property | Value |
| --- | --- |
| Image | 1920 × 1080 |
| Video | H.264 High, limited-range YUV 4:2:0 |
| Frame rate | Constant 60/1 |
| Video frames | 8,090 |
| Video duration | 134.833317 seconds |
| Container duration | 134.837732 seconds |
| Audio | AAC, stereo, 44,100 Hz |
| File size | 388,184,459 bytes |
| SHA-256 | `adc69f4fc69964ae6be6d46da1edbbf111e5e1ef9d00a649ceb6fe599ac24d95` |

The intro is not animation age zero. Timing was established from:

- `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai-en.obj`, `@Combo`, especially
  lines 507–508 and 577–637.
- `pkg/sono-overlay/main_en_16-9_1920x1080.exo`: Combo object `[22]` starts at
  frame 301, giving a zero-based video/object offset of 300 frames (5 seconds).
- `TestingSuite/Sono-Overlay/default.ini`: `offset=216.00`.
- The matching DISPLAYHOLIC chart in
  `TestingSuite/Next Sekai Engine/levels/ABM - DISPLAYHOLIC/` and the PED writer's
  six-decimal seconds serialization in `pkg/sono-overlay/ped.go`.

The observed combo changes and shrink/pop frames agree with:

```text
source_seconds = round(beat × 60 / 110, 6)
event_video_frame = 300 + 216 + source_seconds × 60
animation_age = zero_based_video_frame − event_video_frame
AP pulse time = zero_based_video_frame / 60 − 5
```

The chart's `.679` BGM offset is not added to the PED beat timestamps. These are
reference-video alignment rules, not changes to the application's time model.
The existing source rule that reuses the previous event's age when the new age
is below one frame is preserved. Samples start after that boundary.

For example, combo 2 has event frame 597.81816. Video frames 606–612 therefore
have ages 8.18184–14.18184, not integer ages 8–14. Combo changes were inspected
visually; the additional sampled values are 3, 4, 6, 19, 32, 88, and 112.

## Root causes and changes

1. **AP blend mode:** the previous test/profile assumed the outer
   `obj.setoption("blend", 1)` controlled the transient foreground. Inside the
   AP branch Lua resets blend to **0** before drawing `bN`, then loads and draws
   `pN` without resetting it. The transient foreground must use normal blending.
   Additive blending washed out the foreground gradient even at ages when both
   filters are inactive. Only `combo-event-pop`'s profile blend mode changed.
2. **Backlight was ignored:** Light previously multiplied the foreground RGB.
   It now generates a spatial neutral halo outside the filtered alpha matte
   when `backlight` is enabled. It does not multiply the opaque core in that mode.
3. **Premature clipping:** the former 8-bit intermediate clipped filter energy
   before Lua's final draw opacity. Floating-point emission is now retained
   through that opacity and clipped at destination compositing.
4. **Filter footprint:** the former Glow used the profile blur value directly
   as an output-pixel Gaussian sigma. The calibrated blur and backlight radii
   now scale with the existing source-glyph/draw transform. Geometry is unchanged.

The implementation remains a bounded local pass in `src/render_ui.rs`.
Gaussian passes operate on the matte, with transparent pixels beyond the
surface. No second compositor or per-combo correction exists.

### Calibrated response

All combo values use the same coefficients:

```text
Glow sigma = blur × 0.24 × source_pixel_scale
Glow neutral emission = blurred_alpha × strength × 0.04
Light sigma = 14.6 × source_pixel_scale
Backlight emission = max(blurred_alpha − alpha, 0) × strength × 0.015
```

RGB emission can exceed 255 before draw opacity. Alpha is bounded to 255.
Sigma is bounded to 0.25–64 pixels. The maximum kernel radius is 256 pixels.
The source Glow/Light order, strength curves, transient scale and
`1 − age/14` opacity are unchanged. The non-backlight Light option retains its
foreground multiplier behavior.

These coefficients fit the combined observed response. The Lua strength curves
are nearly proportional, so this video cannot uniquely identify each native
AviUtl kernel. The Gaussian/neutral-emission model is an approximation validated
against this reference, not a claim that AviUtl internally uses these equations.

## Comparison method

60 samples were rendered at 1920×1080 using the checked-in default profile,
normal assets and `render_gameplay_ui`, with normalized event timestamps.
The test isolates the four numeric combo layers; it does not synthesize a
different glyph renderer. Other HUD components are excluded from the metric.

- Fit: combos **2 and 4**, active ages greater than 8 and below 13.5.
- Holdout: combos **3 and 6** at matching ages.
- Additional holdout: **19, 32, 88, 112**, approximately ages 8.5, 10.5, 12.5,
  and 14.5. These include two- and three-digit numbers and different pulse phases.
- Early values also include inactive ages approximately 1, 3, 6, 7 and 14.
- Reference and generated frames use identical combo values and fractional ages.
- No scale, glyph advance, group center, asset opacity or animation-strength
  parameter was fitted. Registration was estimated separately using inactive
  filter frames only: reference shifted by −0.122 px X and −0.158 px Y.
  This shift is analysis-only and never changes production geometry.
- RGB errors use decoded RGB bytes. Single-digit ROI is crop-local
  `x=[175,395), y=[105,295)`; multi-digit ROI is
  `x=[60,500), y=[105,295)`. Crop origin is `(1350,300)`.
  The COMBO title is excluded from both ROIs.
- Core mask: full-alpha transient input above alpha 32. Halo mask: 25-pixel
  dilation minus a three-pixel dilation of that input mask. These are fixed
  geometry masks, not masks chosen from the corrected image.

### Measured active-age errors

MAE is mean absolute RGB-byte error; MSE is mean squared RGB-byte error.
Each row averages equal-sized per-frame ROIs within its group.

| Samples | Before MAE | After MAE | Before MSE | After MSE | MSE reduction |
| --- | ---: | ---: | ---: | ---: | ---: |
| 2/4 fit, 12 active frames | 7.618 | 2.268 | 174.031 | 20.876 | 88.0% |
| 3/6 holdout, 11 active frames | 7.327 | 2.195 | 164.138 | 19.338 | 88.2% |
| 19/32/88/112 holdout, 12 active frames | 7.179 | 2.097 | 171.961 | 19.254 | 88.8% |

The historical “before” includes the old blend and old filters. To isolate the
filter contribution from that blend correction, a separate production capture
uses the corrected normal blend with **effects disabled**, keeping geometry,
assets, timing, scale, and opacity identical:

| Samples | Normal blend, no filters MAE | Calibrated filters MAE | Core MAE before → after | Halo MAE before → after |
| --- | ---: | ---: | ---: | ---: |
| 2/4 | 9.119 | 2.268 | 16.065 → 4.366 | 9.805 → 1.409 |
| 3/6 | 8.786 | 2.195 | 16.537 → 4.335 | 9.959 → 1.645 |
| Multi-digit holdout | 8.726 | 2.097 | 16.813 → 4.160 | 11.122 → 1.795 |

Raw, unregistered after MAE is **2.209 / 2.127 / 2.044**, respectively. The
improvement does not depend on moving glyphs. Inactive-filter output agrees
with the effects-disabled compositor within one RGB code value. Across all
60 samples, the largest per-frame registered after MAE is 2.931.

### Encoding sensitivity

Seven consecutive production frames for combo 2, video frames 606–612, were
placed on a 1920×1080 black canvas and encoded at 60 FPS with libx264 medium,
CRF 18, YUV 4:2:0, then decoded. Incremental encoding error in the numeric ROI
was MAE **1.980**, MSE **14.086**. Encoded-versus-reference MAE was **2.564**,
MSE **21.458**. The encoder settings of the original video are not fully known;
this experiment demonstrates sensitivity, not that all residual error is codec
error. No encoding settings were tuned to optimize the fit.

## Visual evidence

Columns are actual decoded reference, historical production output, corrected
production output, and **raw absolute difference ×4**. Frames are not registered
for these images. Rows identify exact video frame and animation age.

- [Combo 2, consecutive active ages](../artifacts/nle-validation/combo-glow-light-2.png)
- [Combo 19, held-out two-digit counter](../artifacts/nle-validation/combo-glow-light-19.png)
- [Combo 112, held-out three-digit counter](../artifacts/nle-validation/combo-glow-light-112.png)

All eight comparison sheets, original crops, production captures, per-frame
metrics, and the short encoding check are under `target/combo-calibration/`.
They are generated evidence, not production resources.

## Reproduction and regression coverage

Use Python with NumPy, SciPy and Pillow installed in an analysis environment.
They are not application runtime dependencies. The local analysis used
NumPy 2.5.4, SciPy 1.18.1 and Pillow 12.3.0. The script also recognizes an isolated
installation under `target/combo-calibration/python-libs`.

```powershell
python scripts/compare_combo_reference.py --extract --capture --encode
cargo test --lib render_ui::tests:: -- --test-threads=1
cargo test --lib real_automatic_ap_chart_export_contains_gameplay_ui -- --ignored --nocapture
```

The script checks all 60 reference comparisons and fails if any per-frame MAE
exceeds 4.5, any group's active-age MAE exceeds 3.5 or MSE exceeds 40, or filters
fail to improve over the fixed-geometry, effects-disabled output. It checks
inactive-filter stability too. The reference test is deliberate and opt-in
because the large video and chart/reference assets are local ignored fixtures.
Historical baseline PNGs are optional on later runs; they are not overwritten.

Additional unit regressions cover spatial/symmetric backlight, unchanged opaque
core, retained HDR emission, normal AP blend mode, timestamp-derived strengths,
30/60 FPS age semantics, deterministic seeking, original glyph assets and
source-derived center spacing. A stale element-count assertion from the earlier
rank-caption repair was corrected from 28 to 29; no rank layout changed here.
Saved custom profiles are not rewritten. A previously saved copy retaining an
explicit additive combo-pop blend keeps that authored setting; reload the
bundled reference profile to receive its corrected normal-blend configuration.

## Remaining limits

- No exact native AviUtl filter implementation or lossless original is available.
  The remaining colored edge differences and lower-gradient differences are
  visible in the difference images, even where the halo is much closer.
- Applying effects to a logical counter surface is not proof of identical
  per-glyph AviUtl filter/compositing behavior in every overlap case.
- This calibration verifies black-background AP combo samples at 1920×1080,
  60 FPS. It does not establish exact filter parity at every resolution, on
  arbitrary backgrounds, with custom assets, or for FC/milestone burst modes.
- No interactive GUI or packaged-artifact visual session was performed for this
  calibration. Production compositor tests and real MP4 export are reported
  separately from interactive verification.

## Validation outcome

- **Implemented:** calibrated matte-based Glow/Light response, source-scaled
  footprints, HDR emission through draw opacity, normal AP transient blend.
- **Reference-video tested:** all 60 matched count/age cases pass the independent
  pixel limits. The extraction/analyzer and production capture path were run.
- **Visually inspected:** original-reference sheets, corrected/difference sheets
  for single and multi-digit samples, and a real-chart exported HUD crop.
- **Unit tested:** the HUD suite initially reported 47 passed, two failed, one
  ignored. Both failures were the pre-existing 28-versus-29 element-count
  assertions after the rank-caption repair. After updating those assertions,
  both tests passed individually; there are no unresolved failures in that set.
  This includes both new filter regressions and the existing source timing,
  assets, spacing, animation, seeking, score/rank, life and judgment tests.
- **Real-chart export tested:**
  `real_automatic_ap_chart_export_contains_gameplay_ui` passed. It exported
  three consecutive late-pop frames around Baumkuchen combo 46 at chart time
  8.640000 using the normal exporter and Sono VM/interpreter. FFprobe confirmed
  H.264, 1920×1080, 60 FPS, three frames, duration 0.05 seconds. The extracted
  combo crop was inspected. This is a short segment, not a full-chart export.
- **Codec tested:** seven consecutive combo-2 frames encoded/decoded as described
  above. This is a calibration sequence, not another rendering pipeline.
- **Build checked:** `cargo build --release --bin renderer` succeeded, producing
  `target/release/renderer.exe`. The build warned that `rc.exe` was unavailable
  for executable version metadata. No installer/package test is claimed.
- **Formatting checked:** `rustfmt --check src/render_ui.rs` and
  `git diff --check` passed.
- **Not exercised:** interactive GUI preview, clean-machine/package launch,
  every custom saved profile, or lossless/native-AviUtl pixel parity. The two
  separate ignored tests in `gameplay_ui_asset_import_export` were build checked
  but not explicitly run; the real-chart export test above was explicitly run.

The renderer changes are confined to the existing HUD raster-effect path and
the combo pop's blend setting. Earlier unrelated edits already present in
`src/video_export.rs`, the asset import/export tests, and other HUD profile
fields were preserved.
