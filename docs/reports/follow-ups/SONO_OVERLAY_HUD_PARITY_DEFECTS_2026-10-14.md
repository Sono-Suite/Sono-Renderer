# Sono-Overlay HUD Parity Defect Follow-up — 2026-10-14

## Status

The source-verified PERFECT timing/opacity path, combo center-slot spacing, and
combo pop compositing boundary are implemented and tested. This report does not
claim complete visual parity: the reference applies additional Glow and Light
filters to the combo foreground during the latter part of its pop, and those
filters are still not reproduced by the shared HUD compositor.

## Reference trace

The authoritative source is
`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2` and its layout object is
`TestingSuite/Sono-Overlay/pkg/sono-overlay/main2_16-9_1920x1080.object`.

- The layout object's combo object is centered at screen position
  `(960 + 673.5, 540 - 63.5) = (1633.5, 476.5)` for the 1920x1080 layout.
- `sekai.obj2` initializes a transparent temporary buffer at line 549 and copies
  it into the object at line 707. The combo layers around lines 613–675 are
  therefore blended locally in that buffer before being composited over the
  gameplay frame.
- `obj.draw`'s fourth argument is scale and its fifth argument is opacity.
  Sono-Overlay's PERFECT branch uses scale only; alpha is zero before frame 2,
  defaults to one from frame 2 through frame 19, and the image is not drawn from
  frame 20. Between frames 2 and 5, scale is
  `2/3 - (-1.45 + progress/4)^4 * (2/3)`; it is `2/3` from frame 5 through 19.
  The default profile normalizes this scale by the source maximum `2/3`, so the
  screen-space scale is `1 - (-1.45 + progress/4)^4`, then 1.0 through frame 19.
- The continuous AP pulse is
  `min(1, (sin(obj.time * pi * 4/3) + 1) / 2)`.
- Combo digit center slots use
  `shift = -(digit_count/2) + digit_index - 0.5` and a source spacing of 72
  object units. At the authored 150% scale this is exactly 108 output pixels
  between digit centers. Both `b{digit}.png` and `p{digit}.png` use the same
  center slot and source zoom. The base shadow is normal blended with the
  source PNG alpha multiplied by the AP pulse. The base foreground is normal
  blended. The transient shadow is normal blended at
  `ap_alpha * (1 - progress/14)`. The transient foreground is additive, with
  scale `0.70 * (0.5 + progress/16)` and opacity `1 - progress/14`.
- For pop ages greater than frame 8 and less than frame 14, the Lua also applies
  Glow and Light effects to the foreground. For ages 8–11 their strengths are
  `progress` and `progress * 6.5`; for ages 11–14 the source expressions are
  `2.5 * (1 - (progress - 13)/0.5)` and
  `15 * (1 - (progress - 13)/0.5)`. Glow blur is 5 and Light's source parameter
  is 1. These effects remain unsupported in this renderer and are a known
  visual mismatch.

The AviUtl2 API documentation confirms the argument meanings for `obj.draw`,
the additive blend mode, and transparent initialization of a sized temporary
buffer: [AviUtl2 Lua API](https://docs.aviutl2.jp/en/lua/index).

## Defects and corrections

### 1. PERFECT transparency

**Root cause:** an earlier implementation treated the fourth `obj.draw` value
as opacity. The reference defines it as zoom/scale. That made the judgment
appear to fade according to a size curve and also risked multiplying a profile
opacity by a second animation opacity.

**Correction:** the shared frame-state evaluator uses the source's scale curve,
sets opacity to 1 for the visible frames, and hides the sprite outside frames
2–19. The default profile animates only scale and has no second opacity track.
No judgment or scoring code was changed in this pass.

### 2. Combo foreground and shadow

**Root cause:** the combo pop's additive blend had been applied directly against
the gameplay RGB frame. Sono-Overlay instead draws the base shadow, base
foreground, transient shadow, and additive transient foreground into one
transparent temporary buffer, then composites that buffer normally over the
frame. A direct frame-buffer Add can brighten pixels from the gameplay image
under translucent glyph edges.

**Correction:** `UiElement.isolation_group` now lets related UI layers render
into a bounded transparent premultiplied-alpha surface. The four combo-number
layers share `combo-number`; their internal z order is preserved, Add operates
against the accumulated combo pixels, and the completed surface is
source-over-composited onto gameplay. The bound is computed from transformed
element/child bounds, including the animated scale, to avoid allocating a
full-frame scratch image each render. Isolated tests verify the output alpha
composite and render the source `b8` and `p8` glyph layers independently.

**Remaining mismatch:** the source-defined late-pop Glow and Light filters are
not implemented. The reference comparison shows the renderer's pop foreground
is brighter/cleaner than the source video in those late-pop frames. Exact
foreground/shadow pixel parity is therefore not certified.

### 3. Combo digit spacing

**Root cause:** proportional glyph advances do not match the source's fixed
center-slot algorithm. In particular, the `b` shadow textures have different
canvas widths and transparent padding; treating their widths as advances
changes both spacing and apparent centering.

**Correction:** the existing profile's explicit 108-pixel center spacing is
retained for both foreground and shadow fonts. Every digit is centered in its
source-defined slot. This matches the Lua formula at 1920x1080 without applying
arbitrary letter spacing.

| Value | Digit centers (px) | Foreground canvas width (px) | Shadow canvas width (px) |
|---|---|---:|---:|
| `8` | 1633.5 | 123.90 | 187.95 |
| `11` | 1579.5, 1687.5 | 231.90 | 255.00 |
| `48` | 1579.5, 1687.5 | 231.90 | 297.00 |
| `100` | 1525.5, 1633.5, 1741.5 | 339.90 | 384.00 |
| `106` | 1525.5, 1633.5, 1741.5 | 339.90 | 383.48 |
| `111` | 1525.5, 1633.5, 1741.5 | 339.90 | 363.00 |
| `1000` | 1471.5, 1579.5, 1687.5, 1795.5 | 447.90 | 492.00 |

The test `sono_overlay_combo_digits_share_the_source_center_slots_for_all_widths`
checks these positions and measured canvas widths against the real profile
assets.

### 4. Score and life shadow alignment at 1920x1080

**Root cause:** the profile translated the score and life shadows into separate
offset counters. The score shadow was displaced by 3.84 px right and 2.16 px
down; the life shadow by 3.84 px right and 3.24 px down. The bitmap-font path
also normalized the shadow sprite canvases to the foreground height and used
the shadow image's expanded width as its digit advance. That lost the source's
shared digit centers and made the shadows drift across the number.

**Reference:** `sekai.obj2` draws score shadow digits from `score/digit/sN.png`
and then foreground digits from `score/digit/N.png` at the same `obj.draw`
coordinates. The x slots are `-127 + 22 * digit_index` and the outer score
object scale is 150%, giving 33 px between centers. For the life counter, its draw
loop places shadow and foreground digits at identical coordinates using
`563 - (digit_count - digit_index) * 127`; the authored life object scale is
17.33%, giving `127 * 0.1733 = 22.0091` px between centers. Its right-side
anchor stays fixed. No life panel or bar position was changed.

The actual PNG canvases differ: score foreground/shadow are 33x44 and 36-48x56;
life foreground/shadow are 1000x1333 and 1100x1466. The profile now preserves
those source scale ratios, applies the fixed Lua slot spacing, and centers
left/right-aligned slots from the shared canonical font metrics. Shadow and
foreground counter layouts now use the same x/y, anchor, and box geometry.

**Verification:** the regression
`sono_overlay_score_and_life_shadows_share_lua_digit_centers` checks the real
asset dimensions, source-derived scale/spacing, equal score/life digit centers,
and unchanged counter anchors. The 1920x1080 before/after captures render only
these four layers through `render_gameplay_ui`, with score 89532 and life 1000;
they are saved under `target/acceptance-output/score-life-shadow/`. The life
bar/panel geometry test remains unchanged apart from requiring the shadow
counter's anchor to match the foreground.

## Validation evidence

- **Tested:** `cargo test --locked --offline --lib render_ui::tests:: -j 2` —
  39 passed, 0 failed. Includes source timing, deterministic seeking,
  center-slot geometry, independent `b8`/`p8` layers, and bounded isolated
  additive compositing.
- **Build-checked:** `cargo check --locked --offline --lib` passed.
- **Windows release build:**
  `cargo build --locked --offline --release --bin renderer -j 2` passed. The
  resulting GUI/CLI executable is `target/release/renderer.exe` (39,438,336
  bytes). The build warned that `rc.exe` is unavailable, so Windows executable
  version metadata was omitted.
- **Real chart exported:** the release executable rendered 21 consecutive
  Baumkuchen frames at 1920x1080/30 fps. FFprobe reports H.264, 1920x1080,
  30 fps, 0.70 seconds. Rendered RGB sequence hash:
  `a0159ad478d7d5d2f431bccdb042e6057e72a1d0`.
- **Shadow-alignment export:** after rebuilding, the Windows executable rendered
  the Baumkuchen fixture with the corrected default UI profile to
  `target/acceptance-output/sono-overlay-score-life-shadow-aligned.mp4`.
  FFprobe verified H.264, 1920x1080, 30 fps, 0.70 seconds, 1,032,196 bytes;
  the renderer's RGB sequence SHA-1 is
  `79e8e8ea882fc4037d36a67ab19a6f448c092349`.
- **Export frame inspected:** frame 9 at 0.30 seconds was extracted from that MP4
  and visually inspected at full resolution and as a 3x top-HUD crop. The score
  and life glyph shadows remain aligned to their foreground counters in the
  actual encoded output. The isolated compositor before/after comparison is
  `target/acceptance-output/score-life-shadow/score-life-shadow-before-after.png`.
- **Visually inspected:** the consecutive combo sequence, judgment sequence,
  isolated foreground/shadow sequence, direct-Add versus isolated-group
  comparison, and an approximate Sono-Overlay video comparison are saved under
  `target/acceptance-output/animation-inspection/isolated/`.
- **Not pixel-certified:** the checked-in reference video is not a frame-for-
  frame capture of the rendered scene with identical content and event state.
  It demonstrates the source appearance and motion but cannot establish exact
  pixel parity.
- **Clippy:** `cargo clippy --locked --offline --lib -- -D warnings` is not
  green: it reports 145 repository-wide lint errors, including diagnostics in
  unrelated modules and existing warnings in `render_ui.rs`. A filtered review
  of the added isolation/bounds helpers found no new diagnostics. This lint
  command was not run before the changes, so the full count is not a before/after
  comparison.
- **Environment limitation:** the real render succeeded, but the sandbox denied
  writes to Local AppData performance-history and diagnostic paths. Native
  notifications also failed after export. These do not affect the produced
  MP4.

## Render artifacts

- Shadow-alignment real-chart output: `target/acceptance-output/sono-overlay-score-life-shadow-aligned.mp4`
- Earlier 21-frame parity output: `target/acceptance-output/sono-overlay-parity-defects-fixed.mp4`
- Exported frame: `target/acceptance-output/score-life-shadow/export-frame-09s.png`
- Exported top-HUD crop: `target/acceptance-output/score-life-shadow/export-hud-crop.png`
- Direct Add vs isolated group: `target/acceptance-output/animation-inspection/isolated/direct-add-vs-isolated-group.png`
- Reference vs rendered combo sequence: `target/acceptance-output/animation-inspection/isolated/reference-vs-rendered-combo-sequence.png`
- Shadow vs foreground rendered independently: `target/acceptance-output/animation-inspection/isolated/shadow-vs-foreground.png`
- Combo event sequence: `target/acceptance-output/animation-inspection/isolated/combo-isolated-sequence.png`
- PERFECT sequence: `target/acceptance-output/animation-inspection/isolated/judgment-isolated-sequence.png`

## Completion decision

The source-derived PERFECT alpha/scale semantics, 108-pixel digit slots, and
temporary-buffer Add boundary are verified in code and real-chart output. The
combo still differs during the late-pop interval because source-defined Glow
and Light filters are missing. Therefore exact Sono-Overlay combo foreground
parity remains **not complete**; the report does not mark the overall HUD
parity task complete.

## Combo-only follow-up

The subsequent combo-focused pass found that all ten active `/assets/combo/pN.png`
foreground glyphs differed from `TestingSuite/Sono-Overlay/assets/combo/pN.png`,
while the `bN` shadows matched. The foreground family has been replaced with
the exact source PNGs, and only the combo fonts now bypass the 128 px glyph
downsample. A source-byte regression test and a same-segment real MP4
before/after comparison verify this correction. The late-pop Glow and Light
kernels remain unavailable in the local source/tooling and are still a visible
mismatch. Details and artifacts are in
`SONO_OVERLAY_COMBO_REPLICATION_2026-10-14.md`.
