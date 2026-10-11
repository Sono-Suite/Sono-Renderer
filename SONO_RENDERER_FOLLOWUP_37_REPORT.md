# Sono-Renderer Follow-up #37 — Presentation Studio customizability

**Status: partial implementation with the requested template/NLE integration path exercised.** This report distinguishes existing #36 capabilities from changes in this pass. It does not claim completion of every advanced customization item in the prompt.

## 1. Baseline and architecture used

The #36 [`FEATURE_EXISTENCE_AUDIT.md`](FEATURE_EXISTENCE_AUDIT.md) is the baseline. It identifies `CompositionProject` in `src/composition.rs` as the authoritative editable NLE model, `src/gui.rs` as its current editor, and the FFmpeg composition graph as the preview/export compositor. `.sonocomp` is the persisted composition; sources refer to media, tracks own ordering/mute/gain, clips own transforms/effects/masks/keyframes, and transitions are separate overlap objects.

This pass added a template data layer over those production objects. It did not add another timeline, renderer, property evaluator, render backend, or project type for editing. Templates bind inputs and clone ordinary NLE objects. Gameplay MP4s use the same media source binding as any other video.

## 2. Implemented changes

- Added `CompositionTemplate`, `TemplateSourceSlot`, and `TemplateTextParameter` in `src/composition.rs`.
- Added versioned `.sonotemplate` schema 1, bounded 16 MiB input, unknown-field preservation, full production composition validation, and flushed same-directory atomic saves.
- Saving a template replaces imported media paths with typed source slots; it does not embed video/audio/image media. Path-free solid/shape sources are retained as procedural media.
- Applying requires explicit slot bindings and optional text values. It creates new IDs for tracks, clips, groups, linked A/V pairs, transitions, and effects. Failed application is transactional. Resulting clips remain ordinary editable composition objects.
- Text parameter token syntax is `{{param:name|default text}}`; applying resolves it into ordinary clip text rather than creating runtime-bound UI widgets.
- Responsive application scales positions by each canvas ratio and visual/text sizes by the smaller ratio. A recognized full-canvas, unmasked, uncropped video layer is centered and scaled to cover changed aspect ratios.
- The existing GUI now has **Save Template…** and **Apply Template…**, a source-slot selector, text-parameter fields, and a responsive-layout option. Applying is captured as one project undo snapshot.
- Added unit tests for rebind/parameterization/responsive application/native editability/save-reopen/transactional rejection and non-portable font/future schema rejection.
- Added an opt-in FFmpeg test and inspectable PNG/MP4 outputs in [`artifacts/presentation-template-validation`](artifacts/presentation-template-validation/README.md).

## 3. Integration acceptance

| Test | Result | Evidence and boundary |
|---|---|---|
| A — ordinary video, template, parameters, responsive layout, effects/mask/text/graphics/animation/export; no Sonolus initialization | **Passed in production FFmpeg test** | The test creates short ordinary MP4 excerpts, applies the saved template, edits a keyframe, saves/reopens, previews, and exports. Composition JSON has no engine/level/watch state. |
| B — second ordinary source, same template, changed exposed values | **Passed in production FFmpeg test** | Source B is bound to the same template slot and a different title parameter is applied; portrait MP4 is generated and FFprobe validated. The inputs produce visually distinct underlying footage. |
| C — gameplay source through NLE primitives | **Partial / tested as pre-rendered media** | An existing Larp gameplay `render.mp4` is bound to the same ordinary video slot and exported with the same clips/effects/mask/text/shape. The current product has no live Watch-backed composition source adapter; this pass did not add one or synthesize authoritative gameplay data bindings. |
| D — landscape and portrait behavior | **Passed structurally and real-rendered** | The same template authored at 1920×1080 is instantiated at 1080×1920. The background cover rule prevents blank portrait margins. Other insets use uniform scaling and may need manual adjustment for extreme aspect ratios. |
| E — native editability | **Passed in test** | After applying, the test adds a new ordinary text clip and a keyframe, saves/reopens `.sonocomp`, confirms the edit, and exports. |
| F — direct renderer remains usable | **Integration-tested** | `project::tests::saved_project_can_queue_and_render_a_real_fixture` passed after this pass; the direct engine/level/resources/music queue path does not depend on Presentation Studio. |
| G — standalone NLE independence | **Integration-tested** | Ordinary compositions are built and rendered without engine, LevelData, WatchVm, Sono-GCC, or gameplay events. The gameplay composition case uses only an already-rendered MP4. |

### Rendered artifact evidence

All four PNGs were opened and visually inspected. The landscape frame and transition midpoint show the log-screen fixture through a blurred full-frame layer, a larger ellipse-masked inset, a cyan generated ellipse, and outlined text over a translucent box. The portrait ordinary-media and gameplay frames show the same content objects with the background covering the full canvas and the title inside the output bounds. The source test footage itself contains tiny debug/menu UI, and the result is a deliberately diagnostic example rather than a polished creator design. The ellipse mask and cyan ellipse are expected authored geometry. I saw no missing media, unexpected blank edge, or clipped title in these frames. This inspection does not certify GUI interaction or broad visual quality.

FFprobe reports all three exports as H.264, 2.000 seconds: 1920×1080 landscape, 1080×1920 second-source portrait, and 1080×1920 pre-rendered gameplay portrait. This test template has no audio track. PNG SHA-256 values and fixture provenance are recorded in the artifact README.

## 4. Existing customization inventory and #37 remainder

Statuses below refer to the actual code and #36 audit. “Existing” means this pass reused the capability; it does not mean the full expanded request for that family is complete.

| Prompt area | Current state after this pass |
|---|---|
| Text fill, opacity, one outline, shadow offsets, alignment, padded box | **Existing and reused.** The template fixture exercises outline, shadow, alignment, and background. Multiple ordered strokes, shadow color/blur/spread, gradient/image text fill, rounded/gradient containers, wrapping/autofit, tracking/leading, weight variants, rich spans, and reveal modes remain absent. |
| Caption styles and active-word look | **Partial existing.** Short-form title preset and timed subtitles exist. Template text parameters work, but this pass did not add per-word active styling or caption-specific style automation. |
| Transitions | **Existing and reused.** Cross dissolve and dips to black/white with shared easing are implemented. Wipe, slide/push, zoom, and blur dissolve remain absent. The real template test includes a dissolve midpoint preview. |
| Effects and effect stack | **Existing and reused.** Gaussian blur, brightness, contrast, saturation, hue, sharpen, gamma, and chroma key are in the production stack; enable/bypass/reorder and some keyframing already exist. No new effect types or copy/paste/favorites UI were added here. |
| Masks | **Existing and reused.** Rectangle/ellipse, invert, feather, and animated mask geometry exist. Polygon/path, mask combinations, and effect-masked regions remain absent. |
| Blend modes | **Normal alpha only** per #36 baseline; multiply/screen/add/etc. were not added. |
| Shapes and gradients | **Existing shapes:** filled rectangle, ellipse, and line; no linear/radial gradient source or shape stroke/border system was added. |
| Layer frames/rounded corners/transforms | Position, scale, rotation, crop, and masks exist. Dedicated border styles, nondestructive rounded corners, shear/perspective/corner pin, and motion blur remain absent. |
| Keyframe workflow and motion presets | Scalar keyframes and basic hold/linear/ease presets plus a graph editor already exist. This pass did not add animation preset libraries, timing scale/multi-object key operations, or generalized copy/paste attributes. |
| Groups and adjustment layers | User grouping and source-free adjustment/effect clips exist. Group-level transform/effect inheritance and explicit adjustment scope tooling were not expanded. |
| Backgrounds and overlays | Solid/generated shapes plus image/video layers and ordinary text allow manual construction. No gradient background system or specialized callout widgets were added. |
| Reusable presets/templates | Title presets existed. **New:** full composition templates with source slots and text parameters. Independent effect-stack, transition, animation, and layer-style preset catalogs are still absent. |
| Color controls and layout tools | Existing inspector color fields are reused. This pass did not add recent-color management, multi-select alignment/distribution, composition guides, or snapping guides. |
| Audio finishing | Existing clip/track gain, fades, pan, mute/solo, crossfade, and mixer are reused. No audio effects or normalization were added; no DAW features were introduced. |
| Preview/performance/GPU | Preview and export continue through the existing FFmpeg composition semantics; preview is reduced-size. NLE visual composition is CPU/FFmpeg-based, not the gameplay WGPU compositor. No new performance claims or effects were added. |
| Persistence/manifest/watermark | Templates have their own versioned/atomic format. Applying uses regular `.sonocomp` persistence. Existing watermark remains outside template clip styling. The composition template test does not replace or bypass the direct-render watermark pipeline. |

## 5. Feature report checklist

The detailed 53-item request is not fully closed. In particular, this implementation does **not** claim multiple text strokes, gradient rendering, richer transitions, new effects, richer masks/blending, layer borders, new animation/copy-attributes UI, alignment/distribution, or expanded audio effects. Those are remaining product work, not green checkboxes. The reusable-template integration and its tests are the completed part of this pass.

## 6. Validation and evidence classification

- **Build-checked:** `cargo check --locked --all-targets` passed. Windows build script warned `rc.exe` is unavailable, so executable version metadata was not embedded.
- **Formatting:** `cargo fmt --all -- --check` passed after formatting the final changes.
- **Unit-tested:** `cargo test --locked --lib composition::tests` — 63 passed, 0 failed, 2 ignored. The ignored tests require app-discoverable FFmpeg/local fixtures.
- **Real-media tested:** `composition::tests::real_ffmpeg_template_reuse_across_ordinary_sources_portrait_and_gameplay` passed (run explicitly with `--ignored`); it writes PNGs and validates three MP4s with the application-discovered FFprobe pair.
- **Direct renderer integration-tested:** `project::tests::saved_project_can_queue_and_render_a_real_fixture` passed (run explicitly with `--ignored`).
- **Manually visually inspected:** all four generated preview PNGs. No desktop GUI mouse/keyboard session was available, so template save/apply controls were not interactively exercised.
- **Not tested:** packaged-app template usage, other operating systems, arbitrary source aspect ratios, media with source audio routing in templates, long compositions, and live gameplay/authoritative data bindings.
- **No claim:** the full repository test suite, release packaging, or clippy were not rerun for #37.

## 7. Reproduction

See [`docs/PRESENTATION_STUDIO.md`](docs/PRESENTATION_STUDIO.md) for `.sonotemplate` semantics and run commands. The test uses fixed local inputs/time ranges and writes small artifacts to `artifacts/presentation-template-validation/`. Existing files in the artifact directory are refreshed by the explicit validation test; each FFmpeg export itself is staged in the test's temporary directory to avoid output-collision failures.

## 8. Remaining limitations

This pass establishes an editable reusable composition-template foundation and proves it crosses ordinary video, aspect ratios, and a rendered gameplay MP4. It does not deliver the whole “extreme customizability” list as one finished creative-studio UI. The largest remaining architectural/product gaps are live Watch-backed NLE sources, generic responsive constraints beyond the explicit cover/normalized scaling rules, rich text/strokes/gradients, new blend modes and transitions, path/compound masks, and broader reusable style preset authoring. No DAW or arbitrary shader scripting was added.
