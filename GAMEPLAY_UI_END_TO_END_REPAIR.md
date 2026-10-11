# Gameplay UI Reference Repair

## Result

The default gameplay UI now starts from the bundled, asset-backed Sono-Overlay
layout. It is disabled until the user checks **Enable gameplay UI**, but the
profile is populated and resolves its assets without a manually authored JSON
profile or event file. Importing the repository presentation pack recognizes
the complete reference pack and creates the same editable profile. Existing
custom profiles are retained; an explicitly empty profile stays empty.

## Reference investigation

The actual overlay implementation is Lua embedded in
`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2`. The 31 `.lua` files
under the repository's Sono-Overlay tree are AviUtl extension dependencies,
not the main HUD script. `sekai.obj2` contains life, score, combo, and judgment
tracks; `ped.go` generates the chart-forecast `.ped` data consumed by that
script. `main2_16-9_1920x1080.object` supplies the track layers and group
transforms. The production code references none of the `TestingSuite` paths;
it resolves the bundled `assets/presentation-pack.json` and `ui-profiles`
through the application's resource root.

### Source-to-Rust mapping

| Reference behavior | Source evidence | Sono-Renderer implementation |
|---|---|---|
| 1920×1080 reference canvas | `main2_16-9_1920x1080.object`, object groups | `ui-profiles/default.json` uses a 1920×1080 design canvas and normalized layout; `src/render_ui.rs` evaluates it against output dimensions using the profile fit mode. |
| Score panel at upper-left | object `[17.1]` (`main2_16-9_1920x1080.object` around line 847), `X=-583.5`, `Y=-471`, source-space center | `score-panel`: `(x=.02,y=0,w=.345,h=.1322)`; score bar, markers, rank and digits share its region. |
| Life panel at upper-right | object `[15.1]` (around line 730), `X=705`, `Y=-477.5` | `life-panel`: `(x=.755,y=0,w=.225,h=.09375)`; life gauge/value are inside the same upper-right region. |
| Combo at right-center | object `[19.1]` (around line 978), `X=673.5`, `Y=-63.5` | `combo-title` and `combo-value` center at approximately `(.85,.35)` and `(.85,.445)`. |
| Judgment near center | object `[21.1]` (around line 1066), `X=0`, `Y=127.5` | Six mutually conditioned judgment images share center `(.5,.62)`; the selected event label controls visibility. |
| Layer order | object layers 8 (life), 9 (score), 10 (combo), 11 (judgment); overlay sublayers follow their panels | Profile element order and `z_index` put panel artwork before counters and overlay details. |
| Score animation / 30-frame interpolation | `sekai.obj2` score track around lines 419–521: score data is selected by frame and the optional score animation eases the difference over 30 frames | The reference asset profile preserves the score artwork and positions. A live score interpolation is only possible when the UI receives a timed gameplay event/provider; no fabricated Watch score is introduced. |
| Combo digit spacing and transient effect | `sekai.obj2` around lines 528–704: AP/non-AP asset branches, centered 72-unit digit spacing, 0.70 base scale, 8-frame pop and under-14-frame fade/glow/burst | Profile maps the available `p` and `b` glyph families and uses the existing counter/visibility presentation path. The GUI can add the `n` family and map its characters explicitly. Exact frame-for-frame animation parity is not claimed. |
| Life threshold and fill | `sekai.obj2` lines 345–403: life is a track value; danger when `life <= 200`; overflow is an option above 1000 | Default profile's danger style rule now uses `less-or-equal 200`; gauge rendering consumes the existing life source. The profile does not invent a universal life-depletion rule. |
| Judgment timing | `sekai.obj2` lines 710–728: judgment type comes from an overlay setting; two frames hidden, fade-in through frame 5, then held through frame 20 at 30fps | Reference judgment images and event-conditioned visibility are in the existing profile/compositor. An explicit event provider can supply the judgment and timestamp; the UI does not infer hits from draw calls. |

Approximate normalized group centers derived from the `.object` coordinates
are score `(.196,.064)`, life `(.867,.058)`, combo `(.851,.441)`, and judgment
`(.500,.618)`. These are source group centers, not exact visible-pixel
centers; each image has its own local bounds. The profile's normalized image
rectangles were checked against the supplied video frames.

## Gameplay data semantics

`ped.go`'s `CalculateScore` (around lines 242–380) computes a chart forecast
from a fixed archetype weight map, level data, power, score mode, judgment
weight, combo factor, and skill factor. The per-note increment includes
`(power / weightedNotesCount) * 4 * archetypeWeight * judgeWeight * levelFax
* comboFax * skillFax`; the combo factor rises by 0.01 per 100 combo and caps
at 1.1, while active skill can double the factor. This is not an authoritative
universal runtime score formula. `sekai.obj2` reads that forecast from the
`.ped` file, interpolates it, and uses track settings for life and judgment
type. It does not establish that every Sonolus engine publishes those values
as Watch outputs.

The Rust HUD therefore uses the existing `UiGameplayEvent` / configured
scoring adapter when a project supplies one. The reference profile contains
no sample scoring rules. Without an event provider, static panels and
decorative art still render while score/combo/life/judgment values remain
unavailable. This preserves the Watch runtime as the gameplay authority and
does not invent engine-independent score, life, or judgment events.

## Asset and GUI workflow

- `assets/presentation-pack.json` is the production manifest; imported
  references resolve through the normal application resource root.
- The complete official pack imports the 25-element reference layout and
  maps its explicit score/life glyph characters. A generic pack remains an
  asset-only import and is not mislabeled as the Sono-Overlay profile.
- The Gameplay UI editor has a populated profile selector/import path, enable
  control, asset thumbnails, image/sprite selectors, gauge fill and mask
  selectors, per-character glyph assignment, an explicit one-character
  mapping control, editable X/Y/width/height/scale/opacity/layer/anchor
  controls, duplicate/delete, and a renderer-backed design preview.
  Unrepresented glyph families (including the pack's `n` family) can be added
  and assigned to counters.
- Profile and project persistence use the existing versioned profile/project
  paths. The GUI displays a clear error if pack/profile loading fails.
- A legacy unmodified empty `Custom` configuration migrates to the reference
  HUD. Named or explicitly intentional empty profiles are preserved. The
  renderer's last-resort visual fallback does not mutate saved profile data.
- If an empty profile contains an unrelated/incomplete pack, populating the
  HUD switches to the bundled reference pack instead of combining mismatched
  layout and asset identities.

## Visual evidence

`artifacts/nle-validation/gameplay-ui-reference-comparison.png` is a
side-by-side comparison. Left: frame 26s from
`TestingSuite/ui_ref/Larp 64x Preview.mp4`, downscaled to 640×360. Right: a
640×360 real-engine MP4 frame at the corresponding timeline time using the
imported pack and production `render-config` command. The score panel, life
panel, combo location, relative scale, and artwork placement are visibly
consistent.

This is approximate visual comparison, not exact frame parity: the right
frame uses an explicit fixture event to exercise digit/judgment artwork and
supplies the same visible score, rank, life, and combo snapshot as the
reference. Those are test adapter values, not Watch outputs. The gameplay
pixels, render settings, and reference capture are not pixel-identical. The
reference video also includes controls and presentation elements that are
not part of the bundled Sono-Overlay gameplay HUD asset pack. Frames at 12s,
26s, and 42s were inspected to observe static and active states.

`artifacts/nle-validation/gameplay-ui-static-comparison.png` contrasts the
12s source frame with the fresh default/no-event render. Static score/life
artwork stays in place; the source's zero score and 1000 life are absent from
the render because those values are not authored by static artwork and no
authoritative gameplay event provider was configured. The extracted 12s and
42s source frames are also retained as
`gameplay-ui-reference-12s.png` and `gameplay-ui-reference-42s.png`.

## Validation evidence

- **Automated, unit:**
  `cargo test --locked --offline --lib render_ui::tests:: -j 2` — 26 passed.
  This covers default/reference rendering at landscape and portrait sizes,
  no-event static artwork, profile migration, intentional empty preservation,
  imported PNG assets, event-backed values, and profile/project reopen.
- **Automated, real media/export:**
  `cargo test --locked --offline --test gameplay_ui_asset_import_export imported_asset_pack_art_reaches_a_real_gameplay_mp4 -- --ignored --nocapture`
  — passed. It rendered the actual Next RUSH/Larp 64x fixture to H.264 MP4,
  validated it with FFprobe, extracted a frame, and confirmed imported glyph
  artwork reached the output. Frame: `target/gameplay-ui-import-acceptance/sono-overlay-reference-1791578907258-frame.png`.
  The event track in this test is explicitly a fixture adapter, not a claim
  about authoritative Watch scoring.
- **Automated, fresh default/no event export:**
  `cargo test --locked --offline --test gameplay_ui_asset_import_export bundled_default_hud_exports_static_art_without_gameplay_events -- --ignored --nocapture`
  — passed. It enabled `RenderConfig::default()`'s bundled profile, configured
  no event file or scoring rules, exported a valid H.264 MP4, and verified
  reference score-panel pixels in the extracted frame. Frame:
  `target/gameplay-ui-import-acceptance/fresh-default-no-events-1791578815050.png`.
- **Automated, backend parity:**
  `cargo test --locked --offline --lib render::tests::gameplay_ui_event_state_is_composited_by_real_preview_pipeline -j 2 -- --nocapture`
  — passed. It compares CPU/WGPU gameplay state, rendered frames, visible HUD
  pixels against UI-off frames, and repeatability using the real Larp fixture.
  The test adapter now supplies explicit scoring rules because the built-in
  reference profile correctly has no universal scoring configuration.
- **Automated, build check:**
  `cargo check --locked --offline --all-targets -j 2` — passed. `cargo fmt
  --all -- --check` and `git diff --check` — passed.
- **Visual inspection:** exported frame and reference side-by-side inspected;
  approximate placement/art consistency verified.
- **Windows GUI executable:** `cargo build --locked --offline --release --bin
  renderer -j 2` succeeded. Artifact:
  `target/release/renderer.exe`, 38,988,800 bytes, modified
  2026-10-09 21:08:11 UTC. It was launched in workspace-local portable mode
  and remained responsive with a native window handle, then closed after the
  startup check. The exposed window measured only 40×40 pixels in this
  session, so that proves process startup only, not a usable rendered GUI.
- **Interactive GUI:** process startup was verified, but actual click-through
  of import, position/scale/asset assignment, preview, save, and reload was
  not automated or manually exercised in this session. Automated profile and
  project save/reopen tests passed; GUI interaction remains unverified. A
  desktop capture attempt failed with Win32 `The handle is invalid`, and the
  native window could not be captured. This environment did not provide a
  usable screen-capture/interaction surface. The build also warned that
  `rc.exe` was unavailable, so Windows version resource metadata was not
  embedded.

## Known limitations

- The available Watch state does not provide a universal score, combo, life,
  rank, or per-note judgment contract. Those dynamic fields require the
  existing explicit event/scoring provider. No `ped.go` forecast formula was
  promoted into generic renderer semantics.
- The default profile reproduces reference geometry, assets, layer ordering,
  conditions, and selected presentation behavior; exact 30fps score/combo
  animation curve parity has not been established.
- The comparison is downscaled layout evidence, not pixel-perfect parity or
  a proof that the reference video and render used identical runtime/resource
  revisions.
- Physical GUI interaction was unavailable for validation here.
