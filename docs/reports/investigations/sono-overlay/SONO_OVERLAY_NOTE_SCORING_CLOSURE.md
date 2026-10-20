# Sono-Overlay Scoring and HUD Animation Closure

## Outcome

The chart scoring forecast now tolerates beat-bearing non-scoring entities,
keeps recognized notes active when other archetypes are unclassified, and
marks that result best-effort. A missing level rating resolves to `0`. The
existing Gameplay UI compositor now evaluates combo-pop, combo-pulse, and
judgment artwork from immutable scoring-event timestamps, so the same result
is used by arbitrary-time Program Preview frame generation and video export.

This is a chart-derived all-perfect forecast, not live player performance. The
current Watch integration does not provide generic player hit/miss, judgment,
or life-change events.

## Reference investigation and mapping

The Go scoring source is `TestingSuite/Sono-Overlay/pkg/sono-overlay/ped.go`;
the Lua presentation source is `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2`.
The checked-in `TestingSuite/ui_ref/` videos and the reference `.exo` layout
were inspected. The Lua formulas—not hand-selected animation constants—are
used by the HUD state evaluator.

| Reference behavior | Production implementation |
|---|---|
| `ped.go` weight map distinguishes scoring archetypes from zero-weight helpers. `CameraChange` and verified Stage/camera presentation changes do not score. | `src/sono_overlay_scoring.rs` assigns known non-scoring entities zero weight. The `bike.json.gz` fixture confirms entity 266 (`CameraChange`) is ignored. |
| Unmapped archetypes must not be counted as notes based only on a beat field. | Permissive extraction skips unknown archetypes, deduplicates diagnostics by archetype, and marks potentially scoring unknowns as an incomplete best-effort forecast. Strict extraction remains available for diagnostics. |
| `default.ini` and `sekai.obj2` select the configured judgment image; image 1 is PERFECT. | The default profile uses the imported PERFECT artwork and the shared HUD compositor. Judgment is triggered by normalized scoring-note events, not WatchVm memory or engine Draw calls. |
| `@combo` (`sekai.obj2`, around lines 533–695): frame-based pop; a short fading burst glyph; a sub-frame prior-event guard for rapid events. | `apply_note_hud_animations` evaluates the event-time delta at the render FPS. Main scale is `min(1, 0.5 + frames/16)`; pulse scale is `0.5 + frames/16` through frame 14 with opacity `1 - frames/14`. New events restart the pulse deterministically. |
| `@judge` (`sekai.obj2`, around lines 712–735): hidden for 2 frames; fourth-power ease to 2/3 opacity by frame 5; hold to frame 20; then hidden. | The same frame-time thresholds and curve drive profile visibility and opacity. At the event timestamp the judgment is hidden, matching Lua. |
| Reference `.exo` positions: combo object `(673.5,-62.5)` on 1920×1080; judgment `(0,127.5)`. | Existing profile positions `(0.85,0.445)` and `(0.5,0.62)` preserve those normalized placements. Glyphs and judgment images come from the packaged presentation asset mapping. |

The evaluator derives state from the scoring forecast at the requested time;
it does not accumulate animation state while frames are rendered. Simultaneous
notes retain stable chart order and produce one coherent final counter/pulse
state rather than duplicated overlapping glyphs. Backward seeks recalculate
the same state from the immutable event list.

## Scoring and rating behavior

- Rating precedence is explicit user override, valid chart `metadata.rating`,
  then default `0`. Explicit `0` is retained as an override. The effective
  rating and its source are shown in Gameplay Scoring/status text.
- Omitted rating is not an error and cannot stop extraction, combo tracking,
  score simulation, or HUD rendering.
- Known non-scoring entities, including `CameraChange`, BPM/time-scale changes,
  connectors, and verified stage/presentation entities, do not create combo,
  score, or judgment events.
- Unknown archetypes are skipped and reported once per archetype with example
  entity indices. Unknowns that may score make the summary say `best effort`;
  recognized notes continue to contribute. Strict mode remains available for
  diagnostics and rejects unclassified beat-bearing archetypes.
- On the real `bike` fixture, `TransientHiddenDamageTickNote` remains an
  unclassified potentially scoring archetype. It is skipped and disclosed;
  the fixture's recognized scoring notes still produce a usable timeline.
- Existing project/config serialization owns the user override. Runtime
  forecast/diagnostic snapshots are rebuilt from LevelData and saved settings.

## Validation evidence

| Evidence | Result and scope |
|---|---|
| Scoring tests: `cargo test --locked --offline --lib sono_overlay_scoring::tests:: -j 2` | **9 passed.** Covers note classification, strict/permissive behavior, unknown diagnostic deduplication, real `CameraChange` at entity 266, recognized notes alongside unknown archetypes, and default/explicit/metadata/override rating precedence. |
| HUD tests: `cargo test --locked --offline --lib render_ui::tests:: -j 2` | **28 passed.** Covers reference profile bindings, animation before/at/during/after event, exact Lua curve boundaries, consecutive and simultaneous events, backward seeking, asset/placement bindings, and persisted settings. |
| Production Program Preview frame-pipeline test: `render::tests::real_chart_scoring_is_applied_by_the_shared_preview_frame_pipeline` | **Passed** with the real Next Sekai `bike` LevelData and engine through the production preview renderer. It checks rating 0, score/combo changes, event-derived animation state, PERFECT opacity, and changed output pixels. Debug run took about 194 seconds for this chart. This verifies the Preview frame pipeline, not an interactive operator click-through. |
| Real chart MP4 | **Passed.** `target/acceptance-output/sono-overlay-bike-animations.mp4`: H.264, 640×360, 30 fps, 120 frames, 4.000 seconds. Rendered from the real `bike` chart over Watch time 6–10 seconds; BGM/SFX were disabled for this HUD-focused capture. |
| Exported-frame inspection | **Passed.** Extracted actual MP4 frames at local 0.30–0.45s around the first note; at 0.37s the frame shows PERFECT near lower center with score `00003705` and combo `3`. Later inspected frames show score/combo `00012880`/`10` at 1s, `00029112`/`21` at 2s, and `00043580`/`31` at 3s. Frames and comparison images are retained under `target/acceptance-output/`. |
| Reference comparison | **Manually inspected, approximate.** `TestingSuite/ui_ref/` video frames were compared with the exported chart frames. Position, relative scale, imported artwork, combo-at-right placement, and lower-center judgment placement follow the reference layout. The compared gameplay/backgrounds and scoring moments are not the same source/time, so this is not exact frame parity. |
| Formatting and target check | `cargo fmt --all -- --check` and `cargo check --locked --offline --all-targets -j 2` **passed**. The environment emitted a non-fatal Cargo warning that `C:\Users\Admin` could not be canonicalized. |
| Windows executable | `cargo build --locked --offline --release --bin renderer -j 2` **passed**. `target/release/renderer.exe` is 39,115,776 bytes (built 2026-10-10 01:59:55 local). Launching `renderer.exe gui` as a hidden process remained alive for 5 seconds; the smoke-test process was then stopped. This is startup validation, not interactive GUI validation. Cargo reported `rc.exe` unavailable, so executable version metadata was not embedded. |

## Limitations

1. Scoring remains an all-perfect chart forecast. It must not be presented as
   actual play results. The inspected Sonolus runtime does not supply general
   hit/miss events to this subsystem.
2. Judgment animation currently shows the configured/default PERFECT artwork
   for recognized forecast events. Per-note non-perfect judgments require an
   authoritative gameplay result provider.
3. The inspected scoring source does not define generic per-note life loss or
   recovery. Simulated life remains the configured life track unless an
   authoritative event track supplies changes.
4. The `bike` chart contains an unknown potentially scoring archetype, so its
   forecast is explicitly best-effort. No score/combo/judgment is fabricated
   for that skipped entity.
5. Parity was verified at the source formula/profile/layout level and by
   rendered frames. Exact pixels cannot be compared because the checked-in
   reference videos do not match the `bike` chart source and timestamps.
6. The GUI executable was built and startup-smoke-tested, but an interactive
   GUI session was not exercised in this environment. MP4 export and the
   production Preview frame-pipeline test both exercised the shared compositor.
7. `rc.exe` was unavailable, preventing Windows version-resource embedding;
   this did not block executable generation or startup.

## Final status

**Implemented and tested:** permissive note-aware extraction, verified
non-scoring classification, deduplicated best-effort warnings with strict
diagnostic mode, zero default rating with the requested precedence, persisted
rating override, exact timestamp/FPS-based combo and judgment animation
curves, shared Preview/export compositor evaluation, and real-chart MP4 output.

**Not claimed:** actual player result parity, universal arbitrary-engine note
semantics, exact reference-video pixel parity, or interactive GUI click-through.
