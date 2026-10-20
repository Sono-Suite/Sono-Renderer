# Sono-Overlay HUD Regression Investigation and Test-Suite Audit

Date: 2026-10-15

## Executive finding

The closest-to-correct checked-in baseline is commit `c268cf5` (`test`, 2026-10-11) and the corresponding earlier frames in `target/acceptance-output/animation-inspection`. The combo geometry did not need a spacing adjustment: the current reference profile uses 108 px between digit centers at 1920×1080, matching the active `sekai.obj2` formula after the authored 150% object transform.

The confirmed digit-art regression was in the working-tree assets. `assets/combo/p0.png` through `p9.png` had been replaced with the raw authoring files from `TestingSuite/Sono-Overlay/assets/combo`. Those are a different revision from the assets in Sono-Overlay's runnable package. The Lua resolves `PED_DATA.path` to the executable-adjacent `assets` directory (`TestingSuite/Sono-Overlay/main.go:883-884`). The prior glyph test compared against the raw authoring directory, so it accepted the wrong production art. The production digits have now been restored to the packaged runtime set and the test expectation now follows that package set. The copied production files match Git `HEAD` again; no asset files remain modified.

This corrects the identified source-art substitution. It does **not** establish exact rendered-frame parity. A new 1920×1080 crop comparison with combo `46` on both sides still shows an apparent rendered-size/brightness difference. The reference and export do not have matched chart timestamps or animation ages, so that comparison is evidence of a remaining visual question, not a safe basis for another geometry edit. No pixel-parity claim is made.

## Baseline, source, and history

- Repository `HEAD` is `c268cf5beea87c111dddfbc07daeaf08c392509d`. The p-digit files tracked by that commit match the local Sono-Overlay package's `build/assets/combo/pN.png` files. The worktree's pre-correction p-digit files matched `Sono-Overlay/assets/combo/pN.png` instead.
- For example, SHA-256 for `p0.png` is `B7C28CD8CEDC6C1E23E655B3A8C0D1926A0194AFFC5730C34AB80A767DDB87D2` in production and `build/assets`, while raw authoring `p0.png` is `720CED1A66908E123239624D782E5D3303C72D60CBD6489BE84BF27266028CE8`. For `p8.png`, packaged/production is `24A152CFB29B99C5763BE1B6439B56C0AA43956F64DB1978DDD6E392BA821F34`; raw authoring is `9B9C5DBC6DBC6DC267AFB4BB5DE6DB3FA35964E31AA6FC0E8C0A143BF4A3A7AD`. All ten digits were checked for equality with the packaged set.
- `TestingSuite/Sono-Overlay/build` is ignored by Git and exists in this checkout. Its asset provenance is supported by the executable-adjacent lookup in `main.go` and the generated package layout, but it is not a versioned golden-artifact source. The test therefore documents this local test-fixture dependency.
- `animation-inspection/before-*` shows the older, closer art state. The later `combo-before-after.png` makes the raw-authoring substitution visible. The historical report `../../follow-ups/SONO_OVERLAY_COMBO_REPLICATION_2026-10-14.md` says the raw authoring files were correct; that conclusion used the wrong asset provenance and is superseded here. The report is retained as historical evidence.
- No committed production change after `c268cf5` introduced the substitution: the offending asset replacement was present in the dirty working tree. Do not attribute it to a commit without additional history evidence.

### Source-to-render mapping

| Source behavior | Sono-Renderer mapping | Evidence / caveat |
| --- | --- | --- |
| `main.go:883-884`: package assets are resolved beside the Sono-Overlay executable; `sekai.obj2` reads the `.ped` path at lines 59-64 | Presentation pack `combo.pN` resolves to `assets/combo/pN.png` and is loaded into the `combo` bitmap font | Byte comparison covers the local runtime package files and renderer assets. |
| `sekai.obj2:531`: `--track@ap` is a 0/1 AP-combo option; `main.go:287` validates it as boolean; `default.ini:27` sets `ap=1` | Default Presentation Studio profile selects `pN`, `bN`, `pe`, and `pt` | This establishes the source branch, not a universal gameplay achievement detector. |
| `sekai.obj2:586-608,629-635`: `ap == 1` draws shadow `bN`, then foreground `pN`, and selects `pe.png`/`pt.png`; otherwise it uses `nN` and `nt.png` | `combo-shadow-value`, `combo-value`, `combo-glow`, and `combo-title` | Current source contains no FC state or FC-only branch. The older `sekai-v1.obj2` uses a different underscored asset family (`p_N`, `b_N`, etc.); it is a legacy implementation, not proof that current `pN`/`pe`/`pt` are FC-only. |
| `sekai.obj2:624-635`: per-digit shift is `-(count/2)+index-0.5`; x is `shift*72*shift_fax`; object scale is 150% in `main2_16-9_1920x1080.object:966-983` | Source center spacing resolves to `72*1.5 = 108 px`; profile `center_spacing=108` | Exported 46/48 spacing looked acceptable. No spacing change was made. |
| `sekai.obj2:616-676`: frame-counter pop and fade; late AP foreground also receives source Glow and Light effects | Timestamp-evaluated combo base/pulse/pop; shared compositor | Scale/fade behavior is tested numerically. The late Glow/Light filters remain absent from the renderer, so late-pop parity remains incomplete. |
| `sekai.obj2:715-730`: judgment uses source-frame progress; first two frames are transparent, frames 2-5 use the fourth-power scale curve, 5-20 hold at 2/3 scale, and frame 20 is hidden | `judgment_scale`, `judgment_visible`, and `judgment_opacity` evaluated from event time and output FPS | The 30/60 fps formula tests are internal numeric checks; no equivalent full Sono-Overlay sequence was available. |
| `sekai.obj2:363-405`: life uses authored masks/fill regions, overflow and danger artwork, and separate shadow/foreground digits | Default HUD gauge assets and profile masks | Existing tests validate modeled geometry/assets and synthetic fill behavior, not pixel parity against an equivalent source render. The accepted overall life-bar position was left untouched. |

The [AviUtl2 Lua API](https://docs.aviutl2.jp/en/lua/index) defines `obj.draw`'s fourth argument as zoom/scale and fifth as opacity. That supports the source reading above for the judgment draw call; the renderer must still be compared at equivalent source-frame ages to certify visual parity.

## Changes made

1. Restored production `assets/combo/p0.png`…`p9.png` to the packaged runtime art already present at `HEAD`.
2. Changed `sono_overlay_combo_glyphs_match_reference_pngs_at_native_resolution` to `sono_overlay_combo_glyphs_match_packaged_overlay_art_at_native_resolution`. It compares all ten loaded production glyphs, native dimensions, and RGBA bytes to the executable-package assets instead of the different raw authoring revision.
3. Added `packaged_ap_combo_art_reaches_the_production_raster_path`, which renders the AP `p8` through `render_gameplay_ui` at 1920×1080 and checks for the packaged glyph's pale lavender/white palette in the actual raster.
4. Added `overlay_frame_based_animation_ages_are_fps_aware_at_30_and_60` to cover source frame-age formulas at two output frame rates. This complements existing tests; it does not independently prove the rendered animation matches the reference video.
5. Did not alter spacing, score, life, PERFECT, Watch VM, or other HUD parts in this audit.

## Existing HUD test audit

“Real export” below means the test invokes the actual FFmpeg MP4 path. Rendering a frame with the production compositor is marked separately. Classification describes independence of the expected result, not whether a test is useful.

| Test | What it actually verifies / expected-result source | Real export path? | Would catch the observed asset substitution? | Class, blind spots, and recommended follow-up |
| --- | --- | --- | --- | --- |
| `sono_overlay_combo_glyphs_match_packaged_overlay_art_at_native_resolution` | Loaded glyph dimensions and exact RGBA bytes against package output adjacent to Sono-Overlay executable | No; load/asset layer | **Yes**, for the known raw-vs-packaged pN swap | **Partially grounded** in the runnable package/source lookup. Not a committed upstream golden. Keep, and pin package provenance/version when available. |
| `packaged_ap_combo_art_reaches_the_production_raster_path` | AP p8 palette reaches the production `render_gameplay_ui` raster at 1920×1080 | Production compositor, not MP4 mux | **Likely**, for this palette change | **Partially grounded**; threshold/palette assertion is not an exact image comparison and isolates only the base foreground. Add a source-produced matched golden when one can be generated. |
| `sono_overlay_combo_digits_share_the_source_center_slots_for_all_widths` | Computes centers and widths for `8, 11, 46, 48, 100, 106, 111, 1000` using 108 px source slots | No | No | **Self-referential** geometry check: it calls the production metrics/placement helper and reuses profile values. Useful for internal consistency, not visual spacing parity. |
| `sono_overlay_combo_slot_spacing_scales_once_with_the_reference_pop` | Tests world-space centers through profile transform at event ages 0,1,4,8,11,13,14 | No | No | **Partially grounded** formula, but expected transform is manually duplicated from the same source interpretation and helpers. Does not rasterize glyphs. |
| `sono_overlay_ap_combo_assets_follow_the_lua_ap_branch` | Searches Lua source text for branch/file strings and verifies profile asset IDs | No | **No**; it never opened those pixels | **Self-referential/partial**: useful source wiring check, not semantic proof of FC/AP or pixels. Improve with actual asset identity and state-driven compositor assertions. |
| `sono_overlay_score_and_life_shadows_share_lua_digit_centers` | Profile shadow layouts and glyph centers match foreground positions | No | No | **Self-referential** geometry. No shadow opacity, blur, or final-pixel comparison. |
| `combo_foreground_and_shadow_render_as_independent_reference_layers` | Renders each real profile layer separately; checks b8 alpha ceiling and p8 visible peak | Production compositor, not MP4 mux | Partial | **Partially grounded** and useful raster smoke. Does not test exact offset, shape, alignment, or composite result against an independent image. |
| `additive_ui_blend_matches_the_combo_pop_compositing_path` | Synthetic pixel equation for the renderer's Add mode | No | No | **Self-referential** mathematical unit test; no Lua or production asset evidence. |
| `isolated_additive_combo_group_composites_over_gameplay_after_internal_blend` | Synthetic shape layers through isolation-group compositor | Production compositor on synthetic input | No | **Self-referential** infrastructure check; does not use combo artwork or reference colors. |
| `bundled_sono_overlay_hud_renders_at_landscape_and_portrait_sizes` | Profile dimensions and nonempty renders at several sizes | Production compositor, not MP4 mux | Maybe not | **Self-referential** smoke test; no independent reference image. |
| `sono_overlay_reference_geometry_is_preserved_at_16_9_and_adapted_at_4_3` | Expected layout values and responsive transforms from profile/source coordinates | No | No | **Partially grounded** source coordinate check; not raster or matched video comparison. |
| `sono_overlay_ap_glow_and_shadow_use_the_source_sinusoid` | Numeric AP pulse and life-overflow values | No | No | **Partially grounded** formula test, but repeats the formula and checks no pixels. |
| `overlay_combo_and_judgment_curves_are_timestamp_based_and_seekable` | State values at events, during animation, repeated/backward seeking; 30 fps | No | No | **Partially grounded** source formula test; does not detect wrong assets or output compositing. |
| `overlay_frame_based_animation_ages_are_fps_aware_at_30_and_60` | Numeric state values at selected source-frame ages at 30 and 60 fps | No | No | **Partially grounded** numeric test; the expected curve is duplicated in the assertion and no image sequence is compared. |
| `perfect_reference_draw_argument_animates_scale_not_transparency` | Production compositor raster bounds/visibility at selected judgment ages, 30 fps | Production compositor, not MP4 mux | No | **Partially grounded** raster test; it catches absent/incorrect scale visibility but not source-image parity or a 60 fps reference sequence. |
| `reference_hud_pop_and_judgment_are_visible_and_repeatable_at_arbitrary_times` | Deterministic synthetic frame output and visible pixel differences after direct seek | Production compositor, not MP4 mux | No | **Partially grounded** internal rendering test; no independent source frame or full MP4 path. |
| `bundled_life_demo_uses_authored_eased_values_and_bitmap_sign_glyphs` | Demo timeline inputs and glyph behavior | Production compositor fixture, not source video | No | **Self-referential** demo behavior; not a life-bar source parity test. |
| `timeline_keeps_runtime_life_unchanged_and_resolves_numeric_and_category_tracks` | Generic timeline behavior and preservation of runtime life | No | No | **Self-referential** state-semantics test, not artwork validation. |
| `asset_gauge_uses_profile_source_region_mask_and_fraction` | Synthetic gauge source region, mask, and fill fraction | Production compositor on fixture | No | **Self-referential** raster test; the fixture confirms the gauge machinery, not Sono-Overlay's full life output. |
| `real_chart_scoring_is_applied_by_the_shared_preview_frame_pipeline` | Bike chart AP scoring and judgment state are prepared at the shared preview frame boundary; verifies changed RGB | Real chart through production preview-frame renderer; not an interactive GUI session | No | **Partially grounded** integration test. It checks gameplay state and visible frame difference, not exact HUD parity. Passed in this run (204.57 s). |
| `real_displayholic_fixture_generates_deterministic_all_perfect_note_events` | Real chart note extraction, recognized score events, and exclusion of known gimmicks | No rendering | No | **Independent for chart semantics**, not HUD graphics. |
| `baumkuchen_reference_video_score_and_combo_samples_match` | Score/combo samples read from `TestingSuite/ui_ref/baumkuchen x retry now preview.mp4` | No | No | **Partially grounded** in reference media. The reference offset/settings are not known; it validates scoring states, not sprite rendering. |
| `authoritative_pipeline_baumkuchen_events_match_reference_and_sequential_frames` | Real Baumkuchen event stream and sequential/pipelined frame determinism | Internal frame/export pipeline, not necessarily MP4 encoding | No | **Partially grounded** to real chart and sequential state; its assertions do not provide an independent HUD golden. |
| `real_automatic_ap_chart_export_contains_gameplay_ui` (ignored) | Configures a real Baumkuchen export and checks H.264 metadata and score forecast | Yes, when explicitly run | No; it does not decode/measure HUD frames | **Partially grounded** integration test. Ignored in the normal suite, 640×360/24 fps, no extracted frame assertion. |
| `bundled_default_hud_exports_static_art_without_gameplay_events` (ignored) | Exports static bundled art and checks metadata/asset color | Yes, when explicitly run | Partial for some static asset changes | **Partially grounded** real-export smoke. It uses 640×360/2 fps, no events or animation sequence, and no independent frame golden. |

### Tests that gave false confidence

The former raw-authoring glyph comparison was the direct false-confidence case: the implementation and expectation both used the same wrong asset revision. A green result only said that the renderer loaded the copied authoring files correctly. The source-string AP test also passed without checking actual loaded artwork. Center-spacing and animation tests remain valuable internal regression checks, but they cannot certify visible parity because expected values are derived from profile constants and the same production helpers or formulas.

There is no independent source-generated PNG golden or automated video-sequence comparison in the existing suite. The reference MP4 is usable for manual comparison, but its captured scene and unknown event phase make a pixel-for-pixel oracle unsafe without a controlled Sono-Overlay capture.

## AP versus FC conclusion

For the current `sekai.obj2`, the user-facing `--track@ap` control is explicitly named AP combo and constrained to `0` or `1`; the default INI sets it to `1`. In the active script, AP mode chooses `pN` foreground digits, `bN` shadow digits, `pe` glow, and `pt` title. The other branch chooses `nN`, `bN`, and `nt`, with no `pe` glow. The script does not define or inspect a Full Combo state. The old `sekai-v1.obj2` uses underscored `p_N`, `b_N`, and related assets and is a distinct legacy profile. No checked source supports reclassifying the active AP branch's assets as FC-only.

Accordingly, the AP selection was preserved. `combo-special-burst` remains disabled in the default profile. No synthetic FC state or asset interpretation was added.

## Real MP4 and visual inspection

Built `target/release/renderer.exe` with:

```text
cargo build --locked --offline --release --bin renderer -j 2
```

The build succeeded. Windows `rc.exe` was unavailable, so executable version metadata was omitted. The executable SHA-256 is `109D3731CB7DA04D2652EB961C2AAFD7EC1BB91E587CE08F136E708B38712ABE`.

Ran the release executable on the real Baumkuchen chart with the Next Sekai engine/resources, default AP forecast, default HUD, CPU rendering, 1920×1080, 60 fps, start 8.6 s, and duration 0.70 s. The MP4 contains 42 H.264 frames, is 1,091,452 bytes, and has no audio stream because BGM/SFX layers were disabled in the inherited test configuration. FFprobe reports 1920×1080 and `60/1`. The exported score and combo visibly change (`44` → `46` → `48` → `49`), and PERFECT feedback appears. MP4 SHA-256: `513D69F19B11E4C91A13CC430169CFDCA4BA3821807D53B3A4CCCDDB7A273B6B`.

Evidence is under `target/acceptance-output/hud-regression-audit-2026-10-15/`:

- `baumkuchen-ap-1920x1080-60fps.mp4`: real chart export.
- `frames/all-42-frames-contact.png`: all output frames, row-major from frame 0 through 41; shows the actual score/combo/judgment sequence.
- `frames/real-chart-60fps-contact.png`: selected full-frame samples.
- `frames/reference46-left-render46-frame16-right.png`: reference frame at video time 26.0 s on the left and export frame 16 (8.8667 s composition time) on the right, cropped to the combo region. Both show 46. This is an approximate visual comparison only: same chart identifier/count does not prove the same event age, captured note state, or source scene.
- `frames/reference-left-render-right-combo-crops.png`: corresponding 46/48 crop from the earlier arbitrary sample; retained as a mismatched-state comparison, not parity evidence.
- `baumkuchen-ap-1920x1080-60fps.json`: exact export configuration.

The bright-core threshold measurement on the same-count crop was 154×72 px for the reference and 232×126 px for the export. This threshold counts only pixels with all RGB channels ≥220 inside a fixed crop; it is not the full sprite bounding box and is affected by animation/compositing. It confirms the two screenshots are visibly different, but cannot isolate a scale bug. No geometry change was made from that unmatched comparison.

## Remaining verified gaps and limits

- **Combo animation:** frame-age scale/fade logic and event-time evaluation have numeric coverage. The source's Glow and Light effects on the AP foreground during progress frames 8-14 are not implemented. Exact pop parity remains incomplete.
- **PERFECT:** source timing and scale curve are understood and exercised by internal raster/numeric tests. A source-produced sequence with the same event timing and FPS was unavailable, so entrance/hold/disappearance parity is not certified.
- **Combo shadows:** `bN` is the source shadow layer; it shares digit centers with foreground and uses the AP sinusoid plus event fade. The independent-layer test bounds the source alpha but does not verify the composite against an independent source raster. Exact shadow appearance remains unverified.
- **Life:** the source uses `life/v3` assets and authored mask/fill formulas; the overall life-bar position was intentionally left unchanged. Existing tests cover profile geometry and gauge mechanics, not pixel-equivalent reference video frames. Exact life parity remains unverified.
- **Output comparison:** the real export and preview integration both ran; no same-frame source renderer capture was produced. Preview/export semantic agreement is supported by the shared production compositor, but visual parity against Sono-Overlay is not proven.
- **Platform:** this pass used the local Windows checkout and packaged FFmpeg tools. It did not validate an installed clean-machine application.

## Validation performed

- `cargo test --locked --offline --lib render_ui::tests:: -j 2`: **44 passed, 0 failed** (summary from this audit run; focused HUD module).
- `cargo test --locked --offline --lib render::tests::real_chart_scoring_is_applied_by_the_shared_preview_frame_pipeline -j 2`: **1 passed**, real Bike chart, 204.57 s.
- `cargo fmt --all -- --check`: passed; Cargo printed the existing Windows path canonicalization warning.
- `cargo build --locked --offline --release --bin renderer -j 2`: passed; warning that `rc.exe` was unavailable.
- Release CLI real-chart MP4 export: passed; output probed and extracted with FFmpeg/FFprobe, then visually inspected.
- Preserved `target/acceptance-output/animation-inspection`; all new outputs use a separate date-stamped directory.

## Final assessment

**The raw-authoring digit-art regression is corrected and guarded by a test that checks source pixels, plus a production-raster smoke test.** The 108 px slot geometry was not changed because the reported spacing issue was not supported by independent evidence and the real export spacing looked acceptable.

**The HUD is not certified as exact Sono-Overlay parity.** The reference crop still differs visibly from the real export; known late combo Glow/Light effects are missing, and shadow, judgment, and life visual parity lack a matched source-generated frame sequence. The currently available comparisons do not justify guessing at further asset, scale, spacing, or life-position changes.
