# Gameplay UI Reference Parity Closure

## Result

The default gameplay HUD is grounded in the Sono-Overlay source and bundled
reference artwork. The production preview and MP4 export both render this HUD
through the existing gameplay UI compositor. Profile geometry, asset mappings,
and event-backed display behavior are covered by tests and visual artifacts.

There is one important boundary: Watch rendering currently does not publish
authoritative player-result events for score, combo, life, rank, or judgment.
The HUD does not infer them from chart visuals. It can display state supplied
by an explicit event file and configured event-to-HUD rules; without that
provider, it renders static HUD artwork and leaves dynamic results unavailable.
The test MP4 uses clearly labeled fixture events. It proves the integration
path, not that Watch generated player results.

## Reference investigation and mapping

The inspected sources were:

- `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2`: HUD tracks,
  artwork selection, score/combo presentation, life display, and judgment
  timing.
- `TestingSuite/Sono-Overlay/pkg/sono-overlay/ped.go`: chart-forecast score
  generation (`CalculateScore`, starting at line 242).
- `TestingSuite/Sono-Overlay/main2_16-9_1920x1080.object`: reference group
  coordinates and layer ordering.
- `TestingSuite/ui_ref/Larp 64x Preview.mp4`: visual reference. Frames at
  12, 26, and 42 seconds were inspected; the 26-second frame is retained with
  the current parity artifacts.

The Sono-Overlay `CalculateScore` function computes a chart forecast using
chart/archetype weights, level rating, team power, score mode, judgment weight,
combo progression, and skill factor. The Lua HUD consumes forecast data from
`.ped`; this is not a universal record of actual player judgments. The Lua
life and judgment controls are explicit overlay tracks (life defaults to
1000; judgment is selected separately), not values emitted by arbitrary
Sonolus engines. The Lua life renderer switches to its danger artwork at
`life <= 200`; its optional overflow presentation applies above 1000.

| Reference behavior | Reference evidence | Existing Rust representation |
|---|---|---|
| 1920x1080 design space and aspect fit | `main2_16-9_1920x1080.object`; reference video is 1920x1080 at 30 fps | `ui-profiles/default.json` declares a 1920x1080 design canvas and `contain` fit; `src/render_ui.rs` evaluates normalized layout against output size. |
| Score group, upper left | Object group `[17.1]`, source-space center approximately `(-583.5,-471)` | Default profile places score panel near normalized center `(.196,.064)` with score artwork, rank, markers, and digits layered in the same group. |
| Life group, upper right | Object group `[15.1]`, approximately `(705,-477.5)` | Life panel is near `(.867,.058)` with mapped fill, mask, danger/overflow variants, and digit glyphs. Danger styling uses the reference threshold. |
| Combo, right-center | Object group `[19.1]`, approximately `(673.5,-63.5)` | Combo title/value and glyph assets are placed near `(.851,.35)` and `(.851,.441)`. The profile uses ordered element layers. |
| Judgment, near center | Object group `[21.1]`, approximately `(0,127.5)` | Mutually conditioned judgment artwork is centered near `(.5,.618)`. Event age controls visibility and the configured fade-in. |
| Layer order | `.object` layers 8 life, 9 score, 10 combo, 11 judgment | Profile element order and `z_index` preserve panel-before-counter ordering. |
| Score digit interpolation | `sekai.obj2` score track; optional interpolation eases the score delta over 30 frames | Existing state animation/display consumes timestamped state. Exact original animation-curve parity is not claimed for every profile. |
| Combo digits and burst | `sekai.obj2` combo track; centered 72-unit digit spacing, 0.70 base scale, short pop and burst/fade timing | Existing counter and condition path maps imported glyphs. The generic profile system does not reproduce every Lua AP/non-AP burst frame exactly. |
| Life behavior | Lua life track, threshold at 200, optional overflow above 1000 | Existing profile conditions and gauge consume supplied life values; no generic life-depletion formula is assumed. |
| Judgment timing | Lua judgment track; brief initial delay, fade-in, then hold | The profile's event-conditioned image and opacity animation use the existing compositor. The exported judgment frame is checked after the fade begins. |

The coordinates above are approximate source group centers, not visible pixel
centers; individual artwork bounds are offset within each group. Runtime code
loads the application asset manifest and profile. It does not refer to
`TestingSuite` paths.

## Data authority and integration

`src/watch_runtime.rs` reports Watch execution results such as timeline,
callbacks, draws, memory, and scheduled events. It does not currently provide a
generic player score/combo/life/judgment stream. The existing HUD property
system can read raw Watch memory, but interpreting those values as score, life,
or judgment requires an engine-specific, semantics-verified mapping. The
offline pipeline applies explicit `ui.loaded_events` through the existing
`UiScoringConfig` adapter. That adapter matches event names (and optional note
type), applies configured score/life/combo changes, and selects the configured
judgment label. Timestamps are Watch seconds. No chart-visual heuristic or
`ped.go` forecast was promoted into runtime player state.

The GUI now provides an **Event-to-HUD rules** editor in Gameplay UI settings.
It can select event names from the loaded event file, edit initial score,
combo, and life bounds, add/remove rules, set score and life deltas, choose
combo behavior, set a judgment label, and optionally match note type. The
**Use Sono-Overlay starting life (1000)** control is a convenience for the
reference track; it does not assign universal scoring semantics.

Gameplay UI settings also links to **Open Program Preview**. Program Preview
uses the actual configured engine, level, UI profile, and production Watch
frame/compositor path. Its new **Gameplay values rendered in this frame**
section shows state captured in that rendered frame and explicitly reports
when event-backed state is absent or an event file lacks scoring rules. The
separate design preview remains sample-data-only and is labeled accordingly.
The preview is a still-frame renderer, not continuous playback.

## Visual and exported evidence

The visual comparison is approximate layout evidence, not exact frame parity.
The source reference and generated render use different gameplay pixels and
runtime state. Panel placement, relative scale, imported art, score/combo
position, and the judgment region can be compared; a byte/pixel-identical
claim would be misleading.

Artifacts retained for review:

- [`reference-vs-render-hit.png`](../../../artifacts/gameplay-ui-reference-parity/reference-vs-render-hit.png): reference at 26 seconds beside the real-engine render at its corresponding checkpoint.
- [`reference-26s.png`](../../../artifacts/gameplay-ui-reference-parity/reference-26s.png): extracted source reference frame.
- [`render-event-progression.png`](../../../artifacts/gameplay-ui-reference-parity/render-event-progression.png): before event, hit, judgment feedback, and miss/reset contact sheet.
- [`render-hit-feedback.png`](../../../artifacts/gameplay-ui-reference-parity/render-hit-feedback.png): exported judgment artwork after its configured fade-in.
- [`render-before-event.png`](../../../artifacts/gameplay-ui-reference-parity/render-before-event.png), [`render-hit-at-26s.png`](../../../artifacts/gameplay-ui-reference-parity/render-hit-at-26s.png), and [`render-miss-and-combo-reset.png`](../../../artifacts/gameplay-ui-reference-parity/render-miss-and-combo-reset.png): individual rendered states.
- [`event-progression.mp4`](../../../artifacts/gameplay-ui-reference-parity/event-progression.mp4): H.264 640x360, 12 fps, 1.5-second production export using the Larp 64x / Next RUSH fixture and imported reference pack.
- Earlier extracted 12- and 42-second source frames remain in `artifacts/nle-validation/`.

The event-backed MP4 starts at Watch time 25.5 seconds and contains a fixture
hit at 26.0 seconds and fixture miss at 26.5 seconds. It starts with score
601532, combo 46, and life 1000; the hit changes these to 601632, 47, and 880
with PERFECT, and the miss resets combo and changes life to 580 with MISS.
The test extracts frames before the event, during judgment feedback, and after
the miss, and asserts that score, combo, life, judgment, and imported glyph
pixels reach the encoded MP4. These values are test-provider data chosen to
exercise the adapter; only the score/combo snapshot was selected to resemble
the reference frame.

## Validation

| Evidence | Result | Scope and limits |
|---|---|---|
| `cargo test --locked --offline --lib render_ui::tests:: -j 2` | 26 passed | Profile layout, landscape/portrait fitting, import, conditions, and persistence coverage described by the focused suite. |
| `cargo test --locked --offline --lib render::tests::gameplay_ui_event_state_is_composited_by_real_preview_pipeline -j 2 -- --nocapture` | 1 passed | Real Larp 64x / Next RUSH frame session; explicit fixture events advance `(score, combo, life)` through `(0,0,1000)`, `(100,1,880)`, `(300,2,920)`, then `(300,0,620)`. It compares CPU/WGPU state and rendered frames. This is adapter validation, not Watch-generated scoring. |
| `cargo test --locked --offline --test gameplay_ui_asset_import_export imported_asset_pack_art_reaches_a_real_gameplay_mp4 -j 2 -- --ignored --nocapture` | Passed | Explicitly invoked real-media integration test. Produced and probed H.264 640x360 MP4, extracted event frames, and checked visible artwork/state changes. Current output: `target/gameplay-ui-import-acceptance/sono-overlay-reference-1791600959496.mp4`. |
| `cargo test --locked --offline --test gameplay_ui_asset_import_export bundled_default_hud_exports_static_art_without_gameplay_events -j 2 -- --ignored --nocapture` | Passed | Fresh default profile, no event file/rules; static HUD art reaches a valid MP4 while dynamic result state remains absent. |
| `cargo check --locked --offline --all-targets -j 2` | Passed | Build-checked all targets after production GUI/runtime changes. The final added assertions were compiled and run by the real-media test. |
| `cargo fmt --all -- --check` | Passed (exit code 0); Cargo printed a warning that `C:\Users\Admin` could not be canonicalized | The focused Rust changes were formatted before the final test runs. |
| `git diff --check` | Passed | Only line-ending normalization warnings were emitted for the dirty working tree. |
| `cargo build --locked --offline --release --bin renderer -j 2` | Passed | Built `target/release/renderer.exe` (38,984,192 bytes). Windows resource warning: `rc.exe` unavailable, so version metadata was not embedded. |
| GUI startup smoke | Process remained responsive and exposed a native window handle | This was a hidden startup smoke, not visual or interactive validation. Click-through, import, editing, and save/reload in the native window remain unverified. |

The release executable was built after the production GUI/preview changes; the
subsequent changes were test assertions and artifacts only. No physical
interactive desktop surface was available to verify clicking the controls.

## Acceptance status and remaining limitation

- **Implemented and tested:** reference pack/profile mapping; layout and
  multi-aspect evaluation; editable GUI asset/profile controls; explicit
  event-to-HUD rules; event-backed frame state visible in Program Preview;
  real engine render and real H.264 export; event progression and judgment
  fade pixels; no-event static default export.
- **Visually inspected:** approximate reference layout comparison and
  exported hit/judgment/miss states.
- **Build-checked:** Windows release GUI executable.
- **Not verified:** interactive native GUI click-through, exact frame parity
  with the reference video, full Lua animation parity, or authoritative
  in-engine player result generation.
- **Blocked by current data contract:** actual score/combo/life/judgment
  values from live/offline Watch gameplay. There is no such result stream in
  the current Watch report contract. A real authoritative provider or
  engine-specific adapter is needed before the UI can claim actual gameplay
  outcomes. Until then, externally supplied events remain the honest path;
  static artwork remains usable without them.

The core HUD should not be declared fully equivalent to the live Sono-Overlay
workflow yet. Its reference layout and export path are certified to the
evidence above, while true player-result parity remains unimplemented because
the authoritative inputs do not exist in the current Watch integration.
