# Sono-Overlay Combo Replication Follow-up — 2026-10-14

## Status

The combo foreground asset mismatch is corrected and verified against all ten
source digits. The exact Sono-Overlay combo is **not yet fully replicated**:
the late-pop native Glow and Light filters are still not rendered. No 1:1 parity
claim is made.

Scope was limited to the combo indicator at 1920×1080. The score, life,
judgment, gameplay, and responsive-layout behavior was not changed in this
follow-up.

## Source-derived behavior

The authoritative draw loop is
`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2`, around lines 613–675.
It draws each digit into the overlay's transparent temporary buffer using a
fixed center slot:

```text
shift = -(digit_count / 2) + digit_index - 0.5
x = shift * 72
```

The 1920×1080 object scale is 1.5, so adjacent digit centers are 108 output
pixels apart. Shadow `bN.png` and foreground `pN.png` use the same center and
scale. The base shadow is normal blended and pulses with
`min(1, (sin(time*pi*4/3)+1)/2)`. The base foreground is normal blended. During
the 14-frame pop, the shadow is normal blended at pulse × `(1-age/14)` and the
foreground is additively blended at scale `0.70 × (0.5+age/16)` and opacity
`1-age/14`.

For foreground ages `8 < age < 11`, Lua applies Glow first with strength `age`
and blur 5, then Light with strength `age*6.5` and backlight enabled. For
`11 <= age < 14`, it applies the source expressions
`2.5*(1-(age-13)/0.5)` and `15*(1-(age-13)/0.5)`, respectively. The existing
renderer retains the deterministic pop timing, additive isolation, center
slots, and shadow pulse. Those paths were not redesigned here.

## Root cause and correction

The active asset pack resolved its foreground digits from `/assets/combo/pN.png`.
Every one of those ten files differed from
`TestingSuite/Sono-Overlay/assets/combo/pN.png`. All ten shadow files `b0`–`b9`
were byte-identical to the source. The old p8 image had alpha-weighted mean RGB
`(208.4, 202.7, 241.3)`; source p8 is `(168.1, 183.7, 250.9)`. The wrong
foreground artwork therefore made the digits substantially more pink/white
than the Lua package's actual glyphs.

The production `assets/combo/p0.png` through `p9.png` now use the exact Lua
package files. A focused regression test decodes and compares all twenty
foreground and shadow glyphs byte-for-byte against `TestingSuite`.

The image loader also capped every bitmap glyph at 128 px. This reduced the
source 118×150 foregrounds and up-to-181×212 shadow glyphs before drawing. The
combo fonts now opt into a 256 px decode cap, preserving those small source PNGs
at native resolution. Other bitmap fonts retain the 128 px default. Geometry
and the 108 px center spacing were left unchanged.

## Glow and Light investigation

The Lua establishes effect names, order, parameters, and the frames in which
they are applied. It does not contain the native effect kernels. AviUtl2's Lua
API documentation describes `obj.effect` as dispatching a named filter and
setting its parameters, but does not define those filter algorithms
([AviUtl2 Lua API](https://docs.aviutl2.jp/en/lua/)). The installed
`effect.conf` contains legacy parameter-name mappings, not implementations.
The local AviUtl2 installation has no filter source and its installed plugin
directory has no live-render bridge. The local application exposes no
documented headless render command, so a controlled native effect render could
not be run here.

The checked-in `TestingSuite/ui_ref/baumkuchen x retry now preview.mp4` is an
actual reference capture and was used for a visual comparison. It shows the
colored late-pop halo that remains absent from this renderer. The reference
capture and the renderer export can be aligned by combo values and frame count,
but they are not the same captured gameplay frames; this is approximate visual
evidence, not pixel parity. The native Glow/Light kernels remain a concrete
unresolved blocker. No generic substitute was added or described as exact.

## Render evidence

The Windows release executable rendered the same Baumkuchen configuration and
timeline segment before and after the asset correction:

- 1920×1080, 30 fps, six frames, chart start 8.8 s, duration 0.2 s.
- Before and after outputs both contain the combo transition from 46 to 48.
- FFprobe reports H.264, 1920×1080, 30 fps, 0.2 s for the corrected export.
- Before RGB sequence SHA-1: `a26efd3ba2740b38c3ed3484375a7305c5a5808f`.
- After RGB sequence SHA-1: `aea431ad3ff1dc0504e6e3e14c1af98c00f68075`.
- The corrected output still lacks the late-pop Glow/Light halo.

Visual artifacts:

- Before/after exports, six cropped frames each: `target/acceptance-output/sono-overlay-combo-before-after-source-assets.png`
- Actual UI reference and corrected render crops: `target/acceptance-output/sono-overlay-combo-source-reference-vs-rendered.png`
- Corrected six-frame contact sheet: `target/acceptance-output/sono-overlay-combo-after-contact.png`
- Corrected MP4: `target/acceptance-output/sono-overlay-baumkuchen-reference-state-26s-after.mp4`

The before/after sheet shows the old pink-white p glyphs on the left and the
source blue p glyphs on the right. The source-video comparison shows the
remaining colored halo mismatch. The existing combo geometry test covers
`8`, `11`, `48`, `100`, `106`, `111`, and `1000`; it verifies the same centered
108 px digit slots for foreground and shadow.

## Validation

- **Tested:** `cargo test --locked --offline --lib render_ui::tests:: -j 2` —
  40 passed, 0 failed. This includes the exact source-PNG test, center-slot
  widths, additive isolation, and deterministic pop/seek tests.
- **Build-checked:** `cargo build --locked --offline --release --bin renderer -j 2`
  succeeded. `target/release/renderer.exe` was built. The build warned that
  `rc.exe` was unavailable, so executable version metadata was omitted.
- **Real-media tested:** the release executable rendered and finalized the
  six-frame Baumkuchen H.264 MP4 described above.
- **Visually inspected:** before/after combo crop sequences and a comparison
  against the actual `ui_ref` video.
- **Not verified:** exact native Glow/Light output, exact frame parity against
  the reference capture, or a controlled AviUtl2 reference render.

The export process also reported denied writes to the user's LocalAppData
performance-history and diagnostic paths, and a native-notification failure;
the MP4 completed and was verified independently with FFprobe.

## Completion decision

The wrong foreground assets and glyph downsampling are fixed. The combo's
source-defined Glow and Light processing remains missing, and the visual
comparison confirms that it matters. Therefore the exact combo replication
request remains **incomplete** until those native effects can be rendered or
their algorithms are verified and implemented.
