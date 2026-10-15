# Sono-Overlay HUD and Scoring Parity

## Authority and scope

The source of truth is the checked-in Sono-Overlay implementation:

- `TestingSuite/Sono-Overlay/pkg/sono-overlay/ped.go` for archetype weights,
  chart score forecasting, BPM conversion, combo progression, and skill score
  modifiers.
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2` for score, life,
  combo, and judgment presentation and animation.
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/main2_16-9_1920x1080.object`
  and `main2_4-3_1440x1080.object` for authored HUD positions and group scale.
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/default.ini` for the default
  settings (`life=1000`, `min_digit=8`, `ap=1`, `tag=1`, `last_digit=4`,
  `combo_burst=0`, and `judge=1`).
- `assets/presentation-pack.json` and `ui-profiles/default.json` for the
  packaged artwork mapping and the existing compositor profile.

This is a chart-derived All Perfect forecast. Sono-Overlay's Go code creates
forecast data from chart archetypes; it is not an authoritative live player
result stream. The Rust implementation preserves that distinction.

## Source-to-implementation map

| Behavior | Sono-Overlay source | Sono-Renderer implementation |
|---|---|---|
| Note recognition | `ped.go:26-190` weight table; `ped.go:279-305` entity selection | `src/sono_overlay_scoring.rs:409-535` uses the verified weight table, ignores zero-weight known entities, and records beat-bearing unknown archetypes as best-effort diagnostics. `real_bike_fixture_skips_camera_changes_and_keeps_recognized_note_timeline` covers the checked-in `CameraChange` at entity 266. |
| Score | `ped.go:270-272, 327-385`: `levelFax=(rating-5)*.005+1`; per-note standard score is `(power/weightedNotesCount)*4*weight*1*levelFax*comboFax*skillFax`; tournament mode adds 3 per recognized event | `src/sono_overlay_scoring.rs:617-779` simulates the same model once per chart, and `Forecast::state_at` (`:298-308`) reconstructs state by timestamp for arbitrary seeking. Missing rating uses the application policy default of 0; user override and valid metadata take precedence. |
| Combo | `ped.go:341-348` increments for scored entities except `Skill`, `FeverChance`, and `FeverStart`; `comboFax` rises by .01 at each 100-combo boundary and caps at 1.1 | The normalized event timeline increments only recognized positive-weight notes. Skill rows can restart the visual pulse without contributing score/combo. |
| BPM and event boundary | `ped.go` sorts BPM changes and data entities stably by beat; `sekai.obj2:110-158` chooses a row only when `eventTime*fps < frame-OFFSET` | `resolve_event_times` (`src/sono_overlay_scoring.rs:376-407`) preserves source ordering for equal beats and the reference's no-BPM/pre-first-change behavior. `Forecast::state_at` uses the corresponding strict timestamp comparison. |
| Life | `sekai.obj2:345-403`; life is a configurable overlay track, default 1000. It changes artwork at `life <= 200`; optional overflow has its own pulse. `ped.go` does not calculate note-by-note life | The forecast retains the configured initial life (1000 by default); it does not fabricate universal life loss/recovery semantics. Existing configured/manual event data may supply changes. |
| Judgment | `sekai.obj2:712-735`: hidden before 2 frames, eased opacity until frame 5, held through frame 19, then hidden; selected judgment art comes from the `judge` setting | `apply_note_hud_animations` (`src/render_ui.rs`) evaluates the same frame thresholds and fourth-power curve from the event timestamp. The All Perfect forecast selects PERFECT. |
| AP title glow | `sekai.obj2:546`: `min(1,(sin(time*pi*4/3)+1)/2)` | `apply_note_hud_animations` exposes `ap_combo_alpha`, used by the `pe.png` title glow and combo shadow glyphs. |
| Combo base and pop | `sekai.obj2:616-676`: base `shift_fax=min(1,.5+progress/16)`; pop runs for `progress<14`, scale `.5+progress/16`, alpha `1-progress/14`. A fast event can use the previous row while the current row is under one frame old. | The profile already had the timestamp-derived variables and shadow/foreground/pulse layers. A compositor defect meant animated scale changed only the element's layout rectangle; bitmap-font glyphs use independent font metrics and therefore stayed the same size. `render_element` now applies animated scale in the shared affine matrix around the anchor, so counters/text as well as images scale. `combo_burst=0` remains the default. |
| Score award | `sekai.obj2:479-508`: the recent score offset appears briefly; `anim_score=0` is the default, so the full score itself does not interpolate | The transient profile element reads `score_bonus`, but the forecast previously exposed the same authoritative per-event value only as `score_delta`. Forecast-to-HUD adaptation now supplies the expected profile variable. |
| Optional life overflow pulse | `sekai.obj2:371-395`: when overflow display is active and life exceeds 1000, the overflow layer opacity is `(sin(time*pi)+1)/2`; shipped `overflow=0` and ordinary configured max life is 1000 | The overflow gauge consumes timestamp-derived `life_overflow_alpha`; no ordinary 0..1000 life animation is added. |
| Combo glyphs and digit cap | `sekai.obj2:613-659`; `default.ini` sets four trailing digits; AP draws `b` shadow then `p` foreground at 72-unit centers and 0.70 scale | The profile maps `combo.p*.png` and `combo.b*.png`; `UiBitmapFont` carries per-glyph source-size/advance corrections; `format_counter` enforces the four-digit cap. The 16:9 source group center is `(673.5,-63.5)` on a centered coordinate plane. |
| HUD geometry | `main2_16-9_1920x1080.object`: score `(-583.5,-471), scale 150`; life `(705,-477.5), scale 17.33`; combo `(673.5,-63.5), scale 150`; judgment `(0,127.5), scale 150` | `ui-profiles/default.json` contains the source-derived 1920x1080 layout and the source asset bindings. The profile remains rendered by the production gameplay UI compositor. |
| 4:3 geometry | `main2_4-3_1440x1080.object`: score `(-442.5,-489.5), scale 112`; life `(532,-493), scale 12.97`; combo `(505,-47), scale 112`; judgment `(0,95), scale 110` | `ui-profiles/default.json` stores the two source reference canvases and four responsive groups. `responsive_group_matrix` (`src/render_ui.rs:3671-3732`) leaves 16:9 unchanged, maps each group to the authored 4:3 center/scale, interpolates between 4:3 and 16:9, and clamps to the nearest authored layout outside that range. The root `contain` fit keeps artwork undistorted. |

The 16:9 life panel and gauge geometry now follows the source transform
directly: parent center `(1665, 62.5)`, scale `0.1733`, and native panel size
`2560x600`. This yields panel bounds `(1443.176, 10.51, 443.648, 103.98)`.
The gauge rectangle is transformed from the profile's authored source region
`[0.146875, 0.4666667, 0.6, 0.14]`. Numeric life and shadow counters retain
their separate profile coordinates. An earlier verification render used a
pre-correction inline UI snapshot, so it did not contain the profile position
change; the corrected inline snapshot must be used for visual acceptance.
The same responsive life-group transform applies at 4:3.

The profile's responsive group mapping is shared by Program Preview and MP4
export because both call the same `render_gameplay_ui` compositor. It is
profile data; it does not create a second HUD renderer.

## Exactness boundaries

- **Source-derived and covered by tests:** score formula and supported weights;
  zero-weight/non-scoring exclusion; rating precedence; stable event ordering;
  strict timestamp selection; combo and judgment timing curves; AP pulse;
  glyph mapping and source-size compensation; 1920x1080 geometry; 1440x1080
  group centers and scale ratios; intermediate-aspect interpolation; shared
  time-based evaluation for seeking.
- **Reference-specific but not a universal engine rule:** the weight table is
  Sono-Overlay's map. Unknown beat-bearing archetypes are skipped and mark the
  forecast best-effort. An arbitrary engine can use different note semantics.
- **Not defined by the inspected score generator:** universal note-by-note life
  changes and actual player judgments. The implementation leaves life on the
  reference default/configured track and reports chart-derived PERFECT forecast
  values rather than claiming live player results.
- **Not pixel-certified:** the checked-in videos in `TestingSuite/ui_ref/` are
  usable visual references, but the retained comparison uses different
  gameplay footage and/or event state. It can validate relative placement,
  visual size, artwork family, and visible animation, but it cannot establish
  pixel parity for identical chart frames. Sono-Overlay's authoring source and
  layout files are present; a matching reference render with the same chart,
  score events, settings, and timestamps is not available in this validation
  run.
- **Known visual mismatch:** the reference combo script applies a Glow and
  Light effect to foreground combo digits during frames 8–14 of a pop. The
  current shared HUD compositor does not reproduce that effect stack. The
  default AP title pulse, shadow, base glyphs, pop scale/fade, and judgment
  curve are implemented; this per-pop Glow/Light detail remains unverified and
  unsupported.

The retained approximate comparison is
`artifacts/gameplay-ui-reference-parity/reference-vs-render-hit.png`. The
chart render and extracted frames are under
`target/acceptance-output/`; these artifacts must not be described as an
identical-source parity capture.

## Validation record

Run the focused tests with:

```text
cargo test --locked --offline --lib render_ui::tests:: -j 2
cargo test --locked --offline --lib sono_overlay_scoring::tests:: -j 2
cargo test --locked --offline --lib render::tests::real_chart_scoring_is_applied_by_the_shared_preview_frame_pipeline -j 2 -- --nocapture
```

The focused scoring tests use the real Bike, DISPLAYHOLIC, and Baumkuchen
LevelData fixtures where applicable. The shared preview test uses the actual
Bike chart and engine. The prior exported chart MP4 and frame inspections are
recorded in `SONO_OVERLAY_NOTE_SCORING_CLOSURE.md`; a fresh release build and
post-change output probe should be recorded for this pass before claiming
release-artifact validation.

`cargo build --locked --offline --release --bin renderer -j 2` builds the
Windows GUI executable at `target/release/renderer.exe`. The current Windows
toolchain warns that `rc.exe` is unavailable, so executable version metadata
is omitted; this does not prevent producing the executable.

## Follow-up animation and life-position validation

- **Tested:** `cargo test --locked --offline --lib render_ui::tests:: -j 2`
  passed all 33 HUD tests, including deterministic repeated-time rendering,
  visible combo pixel changes, judgment timing, bitmap counter scaling, and
  16:9/4:3 geometry. `cargo fmt --all -- --check` passed.
- **Build-checked:** the Windows release executable was rebuilt after the
  profile correction. The `rc.exe unavailable` warning means only the Windows
  version resource was omitted.
- **Real chart export and visually inspected:** the release executable rendered
  21 consecutive frames from the Baumkuchen chart at 1920x1080/30 fps. FFprobe
  reports H.264, 1920x1080, 30 fps, 0.70 seconds. Audio was disabled in this
  animation-focused render configuration. The contact sheet
  `target/acceptance-output/animation-inspection/combo-contact.png` shows
  changing combo digit scale across note events; the full sequence also shows
  PERFECT feedback changing over time.
- **Life reference comparison:** an earlier MP4 used a stale inline UI
  snapshot, so it did not include the changed profile and was not valid evidence
  of the correction. The corrected inline snapshot was rendered and compared
  with `TestingSuite/ui_ref/baumkuchen x retry now preview.mp4` at 26.0 seconds.
  In the compared 1920x1080 frames, the green fill occupies rows 59–73 in both;
  the LIFE label starts at row 27 in both, and the panel edge at the checked
  right-side region spans rows 29–105 in both. This verifies the measured
  vertical placement of those static elements; the scenes differ, so it is not
  a full-frame pixel-parity comparison. The artifact is
  `target/acceptance-output/animation-inspection/life-reference-at26-vs-render-at8.8.png`.
- **Environment limitation:** the render completed and wrote MP4, but the
  sandbox denied writes to the user's Local AppData performance-history and
  diagnostic paths. These optional records were not persisted. The native
  notification service also returned an error after the output had been
  written.
- **Not interactively tested:** the native GUI Program Preview was not opened
  for manual interaction in this run. Preview and export share the tested
  `render_gameplay_ui` compositor, and the unit test repeated the same
  timestamp render deterministically.
