# Gameplay UI profiles

Profiles are renderer-owned JSON descriptions of the gameplay overlay. They are separate from the desktop application's own controls. The main GUI's **Gameplay UI** tab selects legacy, custom, or off behavior; the existing `render-video --ui-profile <path>` option selects a gameplay UI profile for a CLI render. Shared `RenderConfig` files can also contain the complete UI configuration. Embedding applications can use `RendererUiConfig::load_profile(path)` or `RenderConfig::set_ui_profile(path)`. Profile asset and event-fixture paths are resolved relative to the profile file.

## Included profiles

- `default.json` composes the asset-backed score/rank, life/pause, combo, and judgement HUD. Rank and gameplay values are supplied by the external event/state adapter; the renderer does not infer results from chart objects.
- `minimal.json` is a small text/time example.
- `terminal.json` is a contrasting 4:3 field-monitor example.

The profiles use general image, bitmap-font, gauge, layout, visibility, style, and animation features. Their art and arrangement live in JSON, not engine-specific branches. See [ASSET_USAGE.md](ASSET_USAGE.md) for the inspected asset inventory, exact profile use, and intentionally omitted artwork.

Standalone profile files may include the flat markers
`"format": "sono-renderer-gameplay-ui-profile"` and `"schema_version": 1`.
The bundled markerless examples are treated as legacy schema 0 and remain
readable; the loader rejects an unknown format or future version rather than
silently dropping its fields. When embedded in a saved `RenderConfig`, the
outer renderer-config version owns the nested UI data.

## Visual validation

Run from the repository root:

```powershell
cargo run --offline --example ui_validation
```

The development-only example renders the actual Larp 64x Watch scene through the CPU renderer and applies the production UI compositor to those pixels. It writes a bounded set of 12 PNGs, `MANIFEST.md`, and `SHA256SUMS.txt` under `artifacts/ui-validation/`. The set includes:

- default and event-driven HUDs at 1280x720;
- the same synthetic event state at 1920x1080 and 1024x768;
- before, rising, settled, and expired judgement frames around an externally supplied event;
- a miss state, bitmap-font/animation example, 4:3 profile example, and legacy comparison.

The gameplay-event fixtures are synthetic visual inputs, not authoritative Watch judgments. `MANIFEST.md` records the level, resources, times, resolutions, event inputs, resulting adapter state, and Watch frame counters. Recompute image hashes with:

```powershell
Get-ChildItem artifacts/ui-validation -Filter '*.png' |
  Sort-Object Name |
  Get-FileHash -Algorithm SHA256 |
  ForEach-Object { '{0}  {1}' -f $_.Hash.ToLower(), [IO.Path]::GetFileName($_.Path) } |
  Set-Content artifacts/ui-validation/SHA256SUMS.txt
```

The frames are visually inspected against the supplied screenshot and `TestingSuite/ui_ref/Larp 64x Preview.mp4`. The CPU Watch capture uses the engine-selected `Nexint-V4-BG-min` background and `NexintWaterMark` particles, but the level fixture does not provide the character/music-video layer shown in the reference movie. This mismatch is recorded in the [Follow-up #6 report](../docs/reports/follow-ups/SONO_RENDERER_UI_FOLLOWUP_6_REPORT.md); the HUD PNGs remain production compositor output over real Watch-rendered frames.

## Event adapter boundary

The headless Watch renderer does not receive player input or calculate score, combo, life, judgement, timing accuracy, completion, or failure. A host can supply timestamped events to the profile adapter. Profile rules then update renderer-owned presentation state and variables. The example fixtures demonstrate that boundary without claiming a game result.
