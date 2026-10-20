# Optional main score count animation

## Behavior and controls

In **Gameplay UI → Gameplay values → Gameplay scoring**, enable **Animate main score count**. The persisted HUD/profile field is `animate_score_count` (boolean). It defaults to `false` in both application defaults and partial/older profile deserialization. Existing profiles need no migration. Saving a HUD profile or render configuration preserves the choice through the existing versioned, atomic persistence paths.

With the option off, counters use their existing expression evaluator and formatting. With it on, direct `Score` counter elements (including the supplied score foreground and shadow) use a display-only value. Other expressions, gauges, rank, scoring, combo, life, judgment, award popup geometry and animation remain authoritative and unchanged. Custom calculated expressions, legacy metric overlays and externally constructed HUD states without a prepared gameplay timeline retain their existing behavior.

## Reference

`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai-en.obj`:

- `@Score`, lines 373–383: `Animated Scoring` reads `anim_score`; progress is output-frame age divided by 30.
- Lines 425–430: `current.score - (current.score - previous.score) * (1 - progress)^3`, formatted with `%.0f`.
- `@Data`, lines 81–139: selects the latest event strictly before the render time and the previous row with a strictly earlier event timestamp. Simultaneous rows therefore share the preceding timestamp's score.
- Both supplied `default.ini` files set `anim_score=0`.

The existing Rust HUD had no equivalent `anim_score` option. The score-award popup is a separate animation, whose `score_speed` does not control the main counter.

This implementation uses the reference cubic ease-out over **30 output frames**: 1 second at 30 FPS; 0.5 seconds at 60 FPS. Rapid events restart from the previous authoritative score, as in Lua, which can make a forward step instead of continuously easing from the previous displayed value. Simultaneous recognized manual events are grouped by timestamp. Unknown manual events do not restart the animation. Forecast queries retain the existing strict event-time boundary; manual tracks retain their existing inclusive boundary.

Deliberate differences: only increasing digits animate; decreases/resets apply immediately. The rank/score bar does not animate. Intermediate values use ties-to-even integer rounding, bounded by the authoritative endpoints. After 30 frames the exact authoritative value is restored, including fractional scores supported by the existing renderer. This does not claim complete AviUtl pixel parity.

## Architecture

The existing prepared AP forecast supplies score snapshots. Its previous distinct-time snapshot is found by binary search. Manual event tracks use the existing scoring-rule interpreter; the origin is captured while resolving their existing deterministic timeline. No scoring rules or generated events are added. Opt-in manual export resolves state through the same seeking evaluator; the default incremental export path remains unchanged. Large manual tracks retain linear state-reconstruction cost when this option is enabled.

A derived `display_score_count` value is carried in transient frame-state variables. Only counter presentation consumes it; it is never written back into an event or authoritative score. Preview and export use the same evaluator and compositor. No playback-order-dependent animation state or per-frame event-track generation is introduced.

## Validation

Implemented and unit/integration tested:

- Nine focused tests passed: four new count-up tests plus existing score-award raster, rank alignment, combo/judgment seek, 30/60 FPS animation, and scoring/life tests. One real-export test was excluded from this run and executed separately.
- Default and old-profile pixels match exactly; the opt-in flag survives atomic profile save/load. Tests cover exact final fractional values, simultaneous notes, rapid successive notes, resets and backward/repeated seeks. Authoritative state and all existing animation variables match after removing the new display-only variable.
- Three full-profile 1920?1080 on/off image pairs have differences exclusively in the main score digit rectangle, `[291,81,432,130)`; changed-pixel counts are 4,462 / 4,077 / 3,858. All other pixels, including award popups, rank, life, judgment and combo, are identical within each pair.
- Shared-compositor samples at chart times 8.790000 / 8.906667 / 9.023333 display 63,747 / 65,259 / 67,188 while authoritative scores are 64,656 / 68,630 / 68,630. The default score remains immediate.

Evidence under `target/` (generated, not source assets):

- `optional-score-count-focused-retry.log`: final focused test results.
- `optional-score-count-export.log`: real-chart export results.
- `acceptance-output/score-count-preview-{0,7,14}.png`: enabled shared-compositor HUD samples on black.
- `acceptance-output/score-count-immediate-{0,7,14}.png`: identical states with the option off.
- `acceptance-output/score-count-samples.json` and `score-count-pixel-differences.json`: numeric samples and pixel comparison results.

Real-media/export tested: the explicit `video_export::tests::real_optional_score_count_chart_export` test passed (1 passed, 0 failed). It uses the Next RUSH engine and the real Baumkuchen x Retry Now chart, with the production application-default HUD and only `animate_score_count` enabled. The video-only MP4 is `target/acceptance-output/sono-overlay-scoring-1792093831419.mp4` (546,518 bytes). FFprobe independently confirms H.264, 1920?1080, 60/1 FPS, 15 frames, 0.250000 seconds. Audio layers were deliberately disabled in this visual test.

Visually inspected decoded MP4 frames 0 / 7 / 14 show displayed scores **63,823 ? 65,468 ? 67,305**. Their global timestamps are 8.800000 / 8.916667 / 9.033333. The decoded score-area sequence is `target/acceptance-output/score-count-export-sequence.png`; probe details are in `score-count-export-probe.json`. This verifies actual encoded count-up, rather than just intermediate state values. The full test took 712.76 seconds in the Windows debug harness, including asset loading, paired HUD rendering and full CPU/Watch export; this is not a measurement of count-up overhead.

The HUD-only samples use their exact requested times. The normal export range rounds its starting timestamp up to the next output frame (8.800000 here); these samples must not be treated as matching-time MP4 reference frames.

A transient MSVC LNK1201 debug-symbol write failure occurred on one link; retry succeeded without source changes. Build also reports that `rc.exe` is unavailable, so executable version metadata cannot be embedded in this environment. No interactive native-GUI test, release executable/installer build, packaged-artifact test or matching AviUtl pixel-parity certification is claimed.

## Files changed

- `src/render_ui.rs`: opt-in persisted field, source-derived deterministic display evaluation, counter-only consumption, and four regression tests.
- `src/offline.rs`: enabled manual tracks use the same timeline evaluator as seeking; the disabled export path is retained.
- `src/gui.rs`: checkbox in the existing Gameplay scoring editor.
- `src/video_export.rs`: explicit real-chart count-up export test, using the application default HUD, and paired HUD evidence generation; shares the existing acceptance fixture setup.
- `README.md`: documentation link.
- `docs/OPTIONAL_SCORE_COUNT_ANIMATION.md`: source mapping, behavior, validation and limitations.

No HUD profile geometry, assets, glyph spacing, Glow/Light implementation, scoring rule, Watch code, rank or existing animation formula was changed.
