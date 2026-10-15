# Sono-Overlay Combo Spacing and AP Asset Recheck

## Result

The reported spacing regression is **not reproducible in the current 1920×1080
combo compositor path**. Its profile spacing and transform math already match
the checked-in Sono-Overlay source. I made no production HUD or asset changes;
an arbitrary spacing adjustment would diverge from that source. Regression
tests now cover the source geometry across the requested values and animation
ages.

The asset claim also conflicts with the checked-in Lua: `pe.png`, `pt.png`, and
`pN.png` are selected inside its explicit `ap == 1` branch. The default profile
selects those AP-branch assets, consistent with the renderer's automatically
generated All Perfect track. The non-AP branch selects `nN.png` and `nt.png`.
The checked-in source has no Full Combo state or FC asset branch, so I could not
identify a source-verified FC-only asset to remove or remap.

## Geometry traced from source

`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2` positions each digit
using:

```text
shift = -(digit_count / 2) + digit_index - 0.5
x = shift * 72 * shift_fax
```

The 16:9 layout object in
`TestingSuite/Sono-Overlay/pkg/sono-overlay/main2_16-9_1920x1080.object`
places the combo at output center `(1633.5, 476.5)` and scales its object by
150%. Thus adjacent idle digit centers are `72 × 1.5 = 108 px` apart. The
source pop factor scales both the glyph and its offset from the group center:

```text
persistent counter: 108 * min(1, 0.5 + age_frames / 16)
pop layer:          108 *       (0.5 + age_frames / 16)
```

There is no second spacing multiplier in the renderer. In
`src/render_ui.rs`, `text_glyph_positions` places each bitmap glyph around
center slots; `element_geometry` applies the animated affine scale once around
the element center. At 1920×1080 the responsive matrix resolves to the authored
16:9 layout without an additional compression transform. Foreground and shadow
use the same centers despite their different PNG canvas widths.

The focused test checks output-space centers, group centering, and effective
spacing for `8, 11, 46, 48, 100, 106, 111, 1000`, for foreground, shadow, pop,
and pulse layers at event ages 0, 1, 4, 8, 11, 13, and 14 frames at 60 fps.

## AP versus non-AP asset trace

In `sekai.obj2`, the combo title/glow and foreground digit selection is
conditional:

| Lua condition | Foreground digits | Shadow digits | Tag / glow |
| --- | --- | --- | --- |
| `ap == 1` | `pN.png` | `bN.png` | `pt.png` and `pe.png` |
| otherwise | `nN.png` | `bN.png` | `nt.png`; no `pe.png` |

The default Presentation Studio profile maps `combo-digit-N` to `combo.pN`,
`combo-glow` to `combo.pe`, and `combo-title` to `combo.pt`. That is the AP
branch. `combo-special-burst` is disabled. The regression test verifies the
Lua branch and these profile mappings. The source uses no `FC`/Full Combo
flag, and the available checked-in combo assets contain no separately
identified FC family. I therefore did not reclassify `pe`, `pt`, or `pN` as FC
art or alter the AP mapping based on appearance.

One limitation remains: the profile is the AP presentation profile and does
not currently expose a second non-AP branch selector. This does not cause
FC-only artwork to appear for the automatic All Perfect track, but a manual
non-AP event track is not proven to switch to the Lua `nN`/`nt` presentation.

## Render evidence

Generated compositor output at 1920×1080 and 60 fps:

- `target/acceptance-output/combo-geometry-60fps.mp4` — 128 compositor frames,
  16 frames per requested combo value, with all combo layers isolated over a
  black background.
- `target/acceptance-output/combo-geometry-60fps-contact.png` — source values
  laid out by row and representative pop ages 0, 4, 8, and 12 by column.
- `target/acceptance-output/combo-geometry-60fps-samples.json` — capture
  resolution, frame rate, source slot spacing, and evaluated spacing formula.
- `target/acceptance-output/combo-regression-60fps.mp4` — real Baumkuchen chart
  render; the sequence includes the actual 46→48→49 combo changes.
- `target/acceptance-output/combo-real-chart-frame-46.png` and
  `target/acceptance-output/combo-real-chart-frame-48-pop.png` — extracted
  real-chart frames.
- `target/acceptance-output/combo-regression-60fps-contact.png` and
  `target/acceptance-output/combo-reference-26s-60fps-contact.png` —
  consecutive real-render and reference frames assembled at 60 fps for visual
  inspection. The reference source is 30 fps, so this is a frame-rate
  conversion, not a newly captured 60 fps Sono-Overlay render.
- `target/acceptance-output/combo-reference-versus-rendered-46-crop.png` —
  cropped comparison against `TestingSuite/ui_ref/baumkuchen x retry now
  preview.mp4`.

The reference clip is 30 fps and depicts a different captured gameplay frame
from the renderer export. The side-by-side crop is useful for approximate
appearance comparison, but it is not a matched-input 60 fps parity test. The
synthetic 60 fps sequence verifies actual compositor motion and the tests
verify source-derived geometry numerically; neither substitutes for a
controlled render of the same scene in Sono-Overlay. **No pixel-parity claim
is made.**

## Validation

- `cargo fmt --all -- --check` — passed (Cargo emitted only the existing
  Windows path canonicalization warnings).
- `cargo test --locked --offline --lib sono_overlay_combo_ -j 2` — 3 passed,
  including native-resolution glyph equality and the source-slot/animated
  transform checks.
- `cargo test --locked --offline --lib sono_overlay_ap_combo_assets_follow_the_lua_ap_branch -j 2`
  — 1 passed.
- The real chart MP4 and compositor sequence were rendered and visually
  inspected. The release executable used for the real chart render was already
  built for the preceding combo asset correction; this recheck changed tests
  and documentation only.

## Remaining uncertainty

The local source and profile explain 108 px idle slot spacing and AP asset
selection, and the generated output follows those semantics. Because the
provided comparison is not a matched reference/render capture and no
source-generated 60 fps reference sequence is available, the reported visual
spacing regression cannot be confirmed or declared fixed at pixel level from
these artifacts. If the supplied screenshot represents another Sono-Overlay
revision or a different asset/profile state, that exact version is not present
in the inspected repository evidence.
