# Sono-Renderer Follow-up #7 — watermark system

## Implementation

Added a renderer-owned two-layer watermark system in `src/watermark.rs` and
integrated it into the shared `FrameSession` composition path. Both CPU and
WGPU scene rendering use this compositor after the renderer's existing pixel
output/readback. Video export, single-frame rendering, and direct
`FrameSession` rendering therefore share the same implementation and
renderer-time input.

The fixed attribution is exactly:

> ※Made with「Sono-Renderer」by Nexint.

It is a Rust constant, not a profile field. Its anchor is fixed at bottom-left;
the renderer measures the selected bitmap font and scales the text to fit the
actual output rectangle and safe margin. Profiles can set its bitmap font,
font size, opacity, and margin. The default built-in 5x7 font includes mixed
case and all three special attribution glyphs. Custom fonts use the existing
bitmap atlas/per-glyph PNG model.

`watermark.enabled` is the only master switch. When it is false, both layers
are skipped even if `moving.enabled` is true. When the master is on, the
moving mark has its own subordinate `enabled` field.

The moving layer supports literal text and PNG image/sprite regions, custom
bitmap fonts, tint, opacity, font/image size, scale, rotation, nine anchors,
normalized movement bounds, and three placement layers: after the rendered
gameplay scene but before gameplay UI, above gameplay UI, or topmost. All three
are post-scene composite layers; the moving mark is not inserted behind notes
or particles. Movement supports static placement, bounded
linear motion (bounce/wrap/clamp), repeated or one-shot waypoint paths, and
seeded deterministic random waypoints. Linear and waypoint positions,
visibility duration/repetition, and oscillating size/rotation/opacity all use
absolute renderer time; no wall clock, global RNG, worker-order state, or VM
register is involved. Random waypoint coordinates are derived from an explicit
64-bit seed and segment index with a stable integer mixer.

Image and font atlases are decoded once when the profile is loaded into a
render session and then shared as immutable `Arc` resources across frame
workers. The master-off path skips asset loading and drawing. No new
GPU/CPU synchronization or CLI/profile-selection flags were added.

## Configuration and reproduction

`RenderConfig` now has a serde-defaulted `watermark` field and
`set_watermark_profile(path)`. Existing render configurations deserialize
without needing a watermark section. The low-level
`FrameSession::new_configured_with_sources` constructor remains available and
uses default watermark settings; the explicit
`new_configured_with_sources_and_watermark` method accepts a prepared profile.

The two examples are documented in [watermark-profiles/README.md](../../../watermark-profiles/README.md):

- [validation-image.json](../../../watermark-profiles/validation-image.json) uses the
  existing `assets/judge/1.png` as a moving image with seed `20261006`.
- [validation-text.json](../../../watermark-profiles/validation-text.json) uses moving
  text and a smooth-step waypoint path with seed `42`.

Reproduce the actual Watch-rendered screenshots from the repository root:

```powershell
cargo run --offline --example watermark_validation
Get-FileHash artifacts/watermark-validation/*.png -Algorithm SHA256
```

The validation-only harness uses the real Next RUSH engine, `larp 64x` level,
and `ProSeka Faithful 0.8.4.scp` resources. Its PNGs are in the ignored,
non-production output directory [artifacts/watermark-validation](../../../artifacts/watermark-validation).
The manifest records profile, seed, frame/time, dimensions, and source asset.
`SHA256SUMS.txt` records the generated image hashes.

## Validation

Structural/programmatic checks passed:

- `cargo fmt --all`
- `cargo check --offline --all-targets`
- `cargo test --offline watermark::tests -- --nocapture` — 11 passed.
- `cargo test --offline render_ui::tests -- --nocapture` — 14 passed.
- `cargo test --offline next_rush_baumkuchen_shared_preview_matches_cpu_reference_on_wgpu -- --nocapture` — passed with an enabled, seeded moving text mark in the render configuration; CPU and WGPU/readback output parity remained within the renderer's established tolerance.
- `cargo run --offline --example watermark_validation` — emitted four real-scene PNGs at 1280x720 and 1024x768.

The watermark pose function is deterministic for equal profile, seed, time,
and output dimensions. Tests cover repeated out-of-order time queries, seed
changes, timing windows, animation inputs, both master/moving toggle states,
custom image tint/rotation, a custom bitmap font plus fallback glyphs, and
safe output bounds at 160x90, 320x180, 1024x768, and 1280x720. The renderer
composes watermarks after Watch evaluation, so the movement code itself does
not branch on Interpreter versus Sono-GCC mode. The WGPU parity test exercises
the shared post-render compositor; a separate full-chart Interpreter-versus-
Sono-GCC export comparison was not run for this follow-up.

## Visual inspection and limitations

I opened and inspected all four generated PNGs. The seeded `PERFECT` image
changes position between 24.833s and 25.667s, stays inside the configured
region, and appears at the same normalized path state in landscape and 4:3
outputs. The text fixture shows the configured moving string. The attribution
appears at bottom-left in all images; its mixed case is preserved after adding
lowercase fallback glyphs. I did not see watermark clipping or a missing PNG.

The Larp frame used here contains the real background, lane, particle effects,
and AUTO indicator, but no active notes at the captured timestamp. These
screenshots validate watermark layering and layout over a real rendered scene,
not note/judgement behavior. The default attribution is intentionally small
and rasterized; on this busy background it is legible at native resolution but
has a pixel-font appearance. Profiles can use a custom bitmap font, larger
size, or higher opacity. Arbitrary TTF/OTF loading is not part of the current
renderer font model.

No changes were made to the desktop UI or public CLI flags. `artifacts/` is
ignored by Git, so regenerate the small screenshot set with the command above
when reviewing visual changes.
