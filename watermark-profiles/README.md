# Watermark profiles

Watermark configuration is part of the shared renderer configuration. The
main GUI has a **Watermark** tab, and CLI video renders accept
`--watermark-profile`, `--no-watermark`, `--moving-watermark`, and
`--no-moving-watermark`. Configuration files and embedding applications can
also set the full `WatermarkConfig` directly.

Use `WatermarkConfig::load_profile(path)` or
`RenderConfig::set_watermark_profile(path)` to load one of these JSON profiles.
Asset paths resolve relative to the profile. A profile contains one master
`enabled` switch, fixed-attribution visual settings, and a subordinate
`moving` configuration. The attribution text and bottom-left anchor are
renderer constants and cannot be changed by a profile.

Standalone files may declare `format: "sono-renderer-watermark-profile"` and
`schema_version: 1`. The bundled markerless examples are accepted as legacy
schema 0; unknown formats and future versions are rejected. When the watermark
is embedded in a saved renderer config, that outer schema version owns it.

`validation-image.json` demonstrates a PNG sprite/texture source rectangle,
bounded seeded random waypoints, timing, opacity, scale/rotation animation,
and image tint. `validation-text.json` demonstrates custom text, explicit
waypoints, smooth-step interpolation, deterministic timing, and animation.
They are development fixtures, not production defaults.

Supported content is text and PNG image/sprite regions. Fonts use the
renderer-owned bitmap atlas or per-glyph PNG format (`UiBitmapFont`), so no
platform font rasterizer or TTF dependency is required. Missing custom glyphs
fall back to the built-in 5x7 font, including the exact fixed branding glyphs.

Reproduce the real Watch screenshots from the repository root:

```powershell
cargo run --offline --example watermark_validation
Get-FileHash artifacts/watermark-validation/*.png -Algorithm SHA256
```

The harness writes its inspectable PNGs and deterministic manifest under
`artifacts/watermark-validation/`.
