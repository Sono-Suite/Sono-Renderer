# Sono-Renderer Follow-up #23: Presentation Studio

## Delivery status

**Partial vertical slice; the full Presentation Studio requested by the brief
is not implemented.** This change adds a real, consumed asset-pack registry,
converts the reference HUD profile to stable pack IDs, adds generic numeric and
categorical presentation timeline tracks to the existing compositor, and adds
real-renderer validation frames. It does not add a GUI editor or a general
video/audio composition/export system. Those are substantial required parts
of the brief and remain open work.

## Architecture and boundaries

The implementation extends existing types and paths:

```text
assets/presentation-pack.json
        ↓
PresentationAssetPack (validation, variants, safe path resolution)
        ↓
RendererUiConfig / ui-profiles/default.json
        ↓
UiFrameState + externally supplied gameplay events
        ↓
resolve_presentation_state (numeric/categorical authored tracks)
        ↓
render_gameplay_ui (existing CPU compositor)
        ↓
existing Watch frame / render and export paths
```

`PresentationAssetPack` in `src/presentation_assets.rs` is the sole new
asset-pack manifest loader. It reuses the existing SHA-256 implementation;
profile paths remain relative to the profile and pack sources are confined to
the canonical pack directory. Pack IDs and typed media entries are data. No
Project SEKAI asset names or behavior were added to generic drawing code.

`RendererUiConfig` remains the profile/configuration boundary, and
`UiFrameState` remains the compositor input. Authored values are stored in
separate `presentation_values` / `presentation_categories` maps. Timeline
resolution clones the input frame state and does not mutate Watch or gameplay
state. Existing direct render inputs, jobs, projects, history, cache, telemetry,
and export queue were not replaced or given parallel versions.

One small shared API was exposed: `renderer::sha256_file`, a re-export of the
existing streaming SHA-256 helper, so the validation harness can write hashes
without implementing a second digest. The only new CLI command is the
development-useful asset-pack inspector; no presentation editing CLI was
added.

## Supplied asset inventory

See [the full inventory and role notes](docs/PRESENTATION_STUDIO_ASSETS.md).
The pack contains 199 media files: 195 PNGs and four MP4s. The manifest has 197
logical entries plus two explicit 4:3 video variants. The profile loader
validates only referenced images during profile loading; explicit inspection
hashes and probes the pack. Contact sheets were inspected for numeric and
combo glyph families, and representative video frames were inspected.

The default profile uses original artwork for the score panel/bar/markers,
score and life glyphs, rank art/captions, layered life gauge, combo title and
digits, and judgement labels. The life gauge uses generic image, mask, gauge,
condition, and glyph elements. AUTO is already drawn by the real Watch fixture,
so the profile does not overlay a duplicate. Difficulty/menu/stage art is
preserved in the pack but not forced into the gameplay HUD. `n`/`p`/`b` combo
families remain structural selectable families; only visible/reference-backed
roles are used as semantics. The four MP4s visibly show an ALL PERFECT
celebration, but the generic profile compositor does not yet support video
layers.

## Implemented and checked capabilities

| Area | Status | Evidence / boundary |
|---|---|---|
| Versioned asset pack | Implemented, unit-tested | Schema v1, stable IDs, image/sprite/glyph/video kinds, declared selectors, unknown entry fields preserved, future schema rejected |
| Safe source paths | Implemented, unit-tested | Relative-only paths, canonical pack-directory confinement, bounded manifest/media sizes and image dimensions |
| Aspect-ratio asset selection | Implemented, unit-tested | Exact `pack://id@4:3` variant selector; base source fallback |
| Asset identity | Implemented, tested | Shared SHA-256 code; explicit inspection output contains size/hash/dimensions/alpha and video metadata |
| Default/reference HUD | Implemented, integration-tested by validation harness | `default.json` resolves original `/assets` artwork via pack IDs and existing compositor |
| Timeline scalar values | Implemented, unit-tested, real-renderer sampled | Numeric tracks with Linear, Ease In/Out/In-Out, and bounded cubic Bezier; life demo at 0/5/10/100 seconds |
| Timeline categories | Implemented, unit-tested | Categorical hold/switch tracks with deterministic boundary semantics |
| Runtime/presentation separation | Unit-tested | A supplied Watch/gameplay life value is not changed by timeline evaluation |
| Existing generic UI primitives | Reused | Existing text, bitmap font/glyph map, image, mask/gauge, conditions, transforms, z order, and animations remain in the renderer |
| Reference visual fixture | Real-engine rendered, visually inspected | 16 PNGs from the actual Watch CPU renderer plus production custom-UI compositor; synthetic gameplay events are labeled in `artifacts/ui-validation/MANIFEST.md` |
| Asset-pack CLI inspection | Implemented | `inspect-presentation-asset-pack <manifest> [--json]`; uses safe FFprobe process arguments for declared video entries |
| Legacy/direct workflow | Existing code path preserved; build-checked | No project/profile/asset-pack requirement was added to direct engine + level + resource rendering |

## Visual validation

Reproduce the screenshots with:

```powershell
cargo run --offline --example ui_validation
```

The harness runs the actual Next RUSH Watch runtime against the repository's
`larp 64x` fixture, captures CPU-rendered frames with the engine's selected
background/effect/particle resources, and then calls the production
`render_gameplay_ui` compositor. It saves PNGs and a manifest in
`artifacts/ui-validation/`; `SHA256SUMS.txt` records the PNG hashes. Four
additional images show the authored LIFE profile at 0, 5, 10, and 100 seconds.
Their scene frames are Watch global frames 0, 60, 120, and 1200 at 12 fps.
The explicit synthetic scoring fixture is only a presentation oracle, not an
authoritative gameplay result.

I visually inspected the 1280x720 reference special-hit frame and the LIFE
samples at 0, 5, and 100 seconds. The score/rank artwork, score digits, combo
digits, judgement art, and layered life bar render from supplied assets. The
250-life midpoint correctly switches to the configured danger fill; the
negative endpoint preserves and draws `-100` while the gauge is empty. The
captured real Watch scene uses different background art from the supplied
reference video and has no MV character layer; that is a fixture limitation,
not a claim of pixel-matching the movie. The top HUD is close to the frame
edge, and the LIFE number sits near the end of its gauge; the inspected frames
show no cropped glyphs, but this placement deserves more review at other
aspect ratios for the authored LIFE profile. No interactive GUI was available
or exercised.

The harness also keeps existing 16:9, 4:3, and 1920x1080 reference-profile
renders. These are image outputs, not proof that all responsive layout cases
or video compositions are correct.

## Explicitly not implemented by this change

These required Follow-up #23 capabilities remain deferred, so this report does
not mark the full request complete:

- GUI Presentation Studio workspace: asset browser, preview controls, layer
  list, property inspector, curve editor, drag/drop media import, and edit
  operations.
- General Composition graph, parent/group transform inheritance, cycle
  detection, arbitrary masks/clips, extra blend modes, effect stacks, and
  general image/sprite/video/audio layer types.
- Video decode/seek/trim/rate/placement/compositing and video audio mixing in
  the generic timeline; final export of the supplied MP4 as a placed video
  layer. Asset metadata and frames were inspected, but this is not video-layer
  integration testing.
- Configurable generic score and combo event-rule engines, canonical versus
  presented judgement selection/remapping, mid-chart judgement style tracks,
  and generic rank/difficulty/tournament mappings. The existing external UI
  event adapter remains the current source of optional synthetic/event state.
- Derived expressions, general property-source/modifier system, per-property
  curve editing, generic variable-width glyph metrics/authoring UI, and
  arbitrary glyph-set import workflow.
- Asset preview cache, missing-asset relink workflow, project schema
  integration for compositions/media, profile/editor integration, undo/redo,
  autosave/recovery, and immutable queued composition snapshots.
- End-to-end composed video render through queue/history/manifest/regression
  systems, presentation-specific telemetry and diagnostics, GPU compositor
  surfaces/effects, and media A/V synchronization tests.
- Broad Presentation Studio regression suite and interactive desktop GUI
  testing.

No user-facing editor controls were added for these unavailable capabilities.

## Validation record

| Check | Result |
|---|---|
| `cargo fmt --all` | Build-checked; completed |
| `cargo check --offline` | Build-checked earlier in this pass; successful with existing deprecation/dead-code warnings |
| Asset pack unit tests | Tested: 3 passed (path/schema rejection, aspect selection, dimensions/alpha/hash inspection) |
| Authored LIFE profile test | Tested: passed; profile load resolves the pack and verifies 1000, 250, 0, -100 plus sign glyph mappings |
| Numeric/categorical timeline tests | Tested earlier in this pass: passed |
| `cargo run --offline --example ui_validation` | Real-engine/test-fixture render completed; generated PNGs, manifest, SHA-256 list |
| Visual inspection | Manually inspected four reference/LIFE images; limits noted above |
| CLI pack inspection | Implemented and previously run against the full manifest; output in `artifacts/presentation-studio/asset-pack-inspection.json` |
| `cargo clippy --offline --all-targets` | Build-checked successfully with warnings; existing workspace reports many lint warnings, so this is not a warning-clean result |
| `cargo test --offline --all-targets -- --test-threads=1` | Tested: 405 passed, 4 ignored, 0 failed across library, CLI, integration tests, scheduler tests, and example targets |
| Release build | Not run |
| GUI interactive exercise | Not available / not tested |

Warnings observed from existing repository code include deprecated
`Atomic::fetch_update` usage and an unused `ProgressTracker::pulse` method.

## Architecture-freeze assessment

The change uses the frozen profile/configuration boundary, existing UI
compositor, existing Watch frame path, existing render manifest/content hash,
existing process/FFprobe helper, and existing headless example conventions.
It adds only the asset-pack input schema and timeline data on the existing
profile. It does not add a second Watch evaluator, project system, job queue,
cache, timing model, or video renderer. The full composition/video/editor
boundaries are still undefined and should be designed as extensions of the
existing RenderConfig, export, project, and queue snapshots before a stable
Presentation Studio API is promised.
