# Presentation Studio real-render validation artifacts

These images are produced by the opt-in FFmpeg integration test through the production composition filter graph. The fixture is a standalone ordinary-media composition: it creates no engine, LevelData, Watch VM, Sono-GCC job, gameplay event, or Sonolus resource.

Reproduce from the repository root on a machine with the app-managed FFmpeg/FFprobe pair installed:

```powershell
cargo test --locked --lib composition::tests::real_ffmpeg_standalone_video_audio_project_preview_and_mp4_export -- --ignored --nocapture
```

The main fixture imports three ordinary MP4s (test pattern, red field, blue field), two WAV sine sources, a video+audio MP4, PNG/JPEG stills, and generated solids. It creates a 30 FPS landscape composition, edits linked A/V and multiple tracks, renders preview and source-monitor frames, streams video/PCM preview, and FFprobe-validates landscape, In/Out-range, vertical, and nested-composition MP4 exports. Additional small pixel fixtures exercise effects, masks, transitions, constant source-time modes, and local title presets through the production compositor.

## Main composition frames

- `standalone-landscape-1920x1080.png`: 0.75 s, 1920×1080; diagnostic pattern, overlays, linked camera layer, solid, caption, and color effect stack.
- `standalone-vertical-1080x1920.png`: 0.75 s, 1080×1920; blurred fill behind a centered landscape source.
- `text-background-box.png`: 0.75 s, 1920×1080; the real composition frame with a padded semitransparent caption box. The integration test compares it pixel-by-pixel with the same composition with the box disabled.
- `media-bin-thumbnail.png`: extracted from the ordinary test-pattern video at 160×90 through the bounded FFmpeg thumbnail decoder.
- `standalone-png-jpeg-and-solid.png`: 1.25 s, 1920×1080; imported stills and generated color.
- `source-monitor-video-at-1.25s.png`: source time 1.25 s, 1920×1080; isolated source-monitor render.

The fixture also saves `title-preset-lower-third.png` at 0.75 s (640x360), rendered over a clean generated-color source. `nested-composition-two-level.png` and `nested-composition-two-level-after-edit.png` are 0.5 s parent previews at 640x360: the first uses a red child and the second changes that saved child to green. Both nested images are produced by the real compositor after recursively evaluating current `.sonocomp` files.

## Effect stack frames

These 640×360 frames use a solid-color source. Brightness, contrast, and gamma are enabled in the named order. The two orders have visibly different output pixels. The bypass image returns to the same source pixel as the no-effect composition. Gamma is a typed effect with validation and an inspector control; its FFmpeg implementation uses `eq=gamma`.

- `effect-stack-brightness-contrast-gamma.png`
- `effect-stack-contrast-brightness-gamma.png`
- `effect-stack-bypassed.png`

The animated brightness pair is driven by effect Amount keys at 0.0 s (-0.2) and 0.9 s (+0.4), with frames rendered through the same compositor at 0.1 s and 0.8 s. The real-media fixture also exports an MP4 and compares its decoded 0.8 s pixel with the preview within H.264 tolerance. I inspected both images: the first frame is dark and the later frame is visibly lighter, with a uniform fill and no clipping or missing frame content. This pair demonstrates a deliberately strong keyframed change, not a finished color grade.

- `effect-brightness-animated-start.png`: 0.1 s, 640x360.
- `effect-brightness-animated-end.png`: 0.8 s, 640x360.

| File | Frame / state | Resolution | SHA-256 |
|---|---|---:|---|
| `effect-brightness-animated-start.png` | animated brightness, 0.1 s | 640x360 | `CC650A35DECC2EDBDAE2B7FE414DA21105162BE98B78DCF169FDB877712874D0` |
| `effect-brightness-animated-end.png` | animated brightness, 0.8 s | 640x360 | `C6F9E055BD20D33866A242B19831C92FF5A72AEF34B55220588A6484C7426E69` |

The mask rectangle fixture also runs a brightness effect before masking. The real-media test compares effect-on/off retained pixels and checks that the exterior remains transparent; it does not write a redundant duplicate PNG.

## Mask pixel fixtures

Each mask image is 640×360 at frame time 0.5 s. A generated green source and brightness effect go through the compositor before the mask. The hard rectangle shows its transparent exterior, the feathered rectangle shows a softened boundary, and the inverted ellipse shows a transparent center. The two animation frames are at timeline 0.1 s and 0.9 s and show the rectangle moving horizontally.

- `mask-rectangle-hard.png`
- `mask-rectangle-feather.png`
- `mask-ellipse-inverted.png`
- `mask-animated-before.png`
- `mask-animated-after.png`

## Transition pixel fixtures

All transition frames are 640×360 at 1.5 s, the midpoint of a one-second overlap between solid red and blue clips. The dissolve image visibly blends red and blue; the dip images are fully black and fully white at the midpoint.

- `transition-cross-dissolve-midpoint.png`
- `transition-dip-to-black-midpoint.png`
- `transition-dip-to-white-midpoint.png`

## Source-time mapping fixtures

All source-time images are 640×360 from the synthetic color-bar MP4. The screenshot time is timeline time; the test pattern's embedded clock indicates selected source time.

- `source-time-half-speed.png`: timeline 0.8 s, forward 0.5×, source 0.4 s.
- `source-time-double-speed.png`: timeline 0.8 s, forward 2×, source 1.6 s.
- `source-time-reverse.png`: timeline 0.2 s, reverse through a 1.4 s source window.
- `source-time-freeze-frame.png`: timeline 1.2 s, held source time 1.0 s.

The integration test compares multiple render times to prove that forward/reverse frames advance and freeze frames remain pixel-identical, then exports a reverse-mapped MP4 and checks its duration with FFprobe.

## Visual inspection

I inspected the main frames, the lower-third title, both nested-child frames, all mask/transition/source-time frames, and the three effect-stack frames. The isolated tests show the expected mask bounds/feather/inversion/motion, dissolve mix, black/white dip midpoints, source-clock mapping, visibly distinct effect order, unmodified bypass color, and child edit propagation. The synthetic landscape/portrait examples are intentionally busy with overlapping diagnostic layers; they are evidence for composition behavior, not a polished edit or approval of the current GUI's visual design. The first title artifact overlapped a fixture caption and was replaced with the clean-color title frame. No missing source or unexpected clipping appeared in the isolated fixtures. Interactive GUI editing was not tested.

## Deterministic image identity

SHA-256 values identify the generated PNGs from the current Windows FFmpeg fixture run. Different FFmpeg builds can change decoded pixels or PNG bytes.

| File | Frame / state | Resolution | SHA-256 |
|---|---|---:|---|
| `standalone-landscape-1920x1080.png` | 0.75 s | 1920×1080 | `bf22debee14252d650cfd8286447dec07b97ab1742ce37178fbac8ebd08291a6` |
| `standalone-vertical-1080x1920.png` | 0.75 s | 1080×1920 | `a49355beac1c48cbf2d0bf51daaa48ff952a8d20a0f04af026b975d352d4be2d` |
| `text-background-box.png` | 0.75 s | 1920×1080 | `bf22debee14252d650cfd8286447dec07b97ab1742ce37178fbac8ebd08291a6` |
| `media-bin-thumbnail.png` | source thumbnail | 160×90 | `495bd973e2c3883194ce73ecddef137df4ceaf04ed4e53a47f3713a9f00b0129` |
| `standalone-png-jpeg-and-solid.png` | 1.25 s | 1920×1080 | `c38496b019db5edbae1df7af41597470ad3734fe73f32b89a7a49eb8a3958fb6` |
| `source-monitor-video-at-1.25s.png` | source 1.25 s | 1920×1080 | `d7014ee4b4941a24bbb04590a7bb6960ca00df3ccb1409cc95d2f0de56c78f6b` |
| `effect-stack-brightness-contrast-gamma.png` | brightness → contrast → gamma, 0.5 s | 640×360 | `b888d5d296a6ffd33f4da06d8333dfb2d55f6466a5c11b8c932cd1fe1b968a68` |
| `effect-stack-contrast-brightness-gamma.png` | contrast → brightness → gamma, 0.5 s | 640×360 | `124d5c6e823e25095ab91fa6de351d57914df82c8d01565f456c7b024cb45790` |
| `effect-stack-bypassed.png` | all effects disabled, 0.5 s | 640×360 | `2f2ab50ab07e207c14a5c8abe205dd638e206436bb0617328d3fb18919edb829` |
| `mask-rectangle-hard.png` | rectangle, hard edge, 0.5 s | 640×360 | `b2ca8ad190ed8cf96438ad1fd7b8947e94f575a89f3747ad67d26b52725a2c0f` |
| `mask-rectangle-feather.png` | rectangle, feather 0.08, 0.5 s | 640×360 | `930eb2d1ce52189d2bd076974363c530f889ac30cccec1f41ba8475cfae5e230` |
| `mask-ellipse-inverted.png` | inverted ellipse, 0.5 s | 640×360 | `93f7c241176763bdf0ecb308b31557bae77bdc255315b747bf054067c3b4ab7f` |
| `mask-animated-before.png` | animated X, 0.1 s | 640×360 | `7e8769e5082f7efdc5a71006ae396d58043c57ec9e5f81342da7fbb503b27e2c` |
| `mask-animated-after.png` | animated X, 0.9 s | 640×360 | `b75efbcf87bc709fc22bbb4ea7237134cc45d610fe48eabfd78f68be73732a33` |
| `transition-cross-dissolve-midpoint.png` | dissolve midpoint, 1.5 s | 640×360 | `acd6579fbc90c4d7be2cf98d9dc9c9ac9e8e554e89553028324bd6e9e12333b2` |
| `transition-dip-to-black-midpoint.png` | black midpoint, 1.5 s | 640×360 | `86bebfe84bb94aab1ced7a77a9f16470ebc4891808690c69bf9c2470d2713d8e` |
| `transition-dip-to-white-midpoint.png` | white midpoint, 1.5 s | 640×360 | `68c2d9171ece439a6b809b599d1776d5a6ff2c20b9f099468a86a84ef46d7224` |
| `source-time-half-speed.png` | forward 0.5×, timeline 0.8 s | 640×360 | `3e7fa331e91ac4259f4a919983e0d2de49d675e34821b8791e25ef8d3b6243f4` |
| `source-time-double-speed.png` | forward 2×, timeline 0.8 s | 640×360 | `e3adc850f72d95612282aca68abe211f26dedd82cfb12b6a109e24ca3c465d09` |
| `source-time-reverse.png` | reverse, timeline 0.2 s | 640×360 | `5136037b19c93b89d528b486d0d5206f196d0da5a9daf49c6cbc2b89f9b5dd2c` |
| `source-time-freeze-frame.png` | freeze source time 1.0 s | 640×360 | `80ba71d59750d3c5d9932e0cd95548c793985e96ca35dcea09d4f8e465113974` |

## Additional follow-up validation frames

These files were generated by the same opt-in production FFmpeg integration test:

- `title-preset-lower-third.png`: project-local lower-third preset on a clean blue 640x360 canvas; the title is inside the frame.
- `generated-rectangle-ellipse.png`: 0.5 s at 640x360; a generated cyan rectangle and yellow ellipse are normal visual sources over a dark solid. Transparent corners preserve the underlying layers.
- `transition-ease-in-out-quarter.png`: 1.25 s at 640x360, one quarter into a one-second red-to-blue dissolve using Ease in/out. The fixture also compares the center pixel with linear progress and asserts the eased blue contribution is lower.
- `transition-trimmed-retimed-preview.png`: 1.5 s at 640x360; source-trimmed clips at 0.5x and 0.75x playback overlap in a cross-dissolve. The fixture exports the same composition, decodes the matching MP4 frame, and checks the center pixel within H.264 tolerance.
- `nested-composition-two-level.png`: 0.5 s at 640x360, parent resolves a red grandchild composition.
- `nested-composition-two-level-after-edit.png`: same frame after saving the grandchild green; the parent resolves the new saved child state.

I visually inspected these frames. The shape frame has clean transparent areas and no unexpected clipping. The quarter-progress eased frame remains mostly red, as expected for ease-in/out before the midpoint; pixel validation proves the changed blend ratio (the transition midpoint is shown in the existing dissolve artifact). The trimmed/retimed transition shows the source test pattern blended with red; its embedded source clock is about 0.63 s, consistent with a 0.25 s source in-point plus 0.5 s at 0.75x. The lower-third is fully inside the canvas. Nested outputs are full-frame red then green.

| File | Frame / state | Resolution | SHA-256 |
|---|---|---:|---|
| `title-preset-lower-third.png` | 0.75 s | 640x360 | `c3ffb0661acc4fe585fb7c2dac040080522815dcef4e035f7d82d754397e774d` |
| `generated-rectangle-ellipse.png` | 0.5 s | 640x360 | `9478efa3a65fd02391a58c78f4a446ab1914a30df86e19e79938922dc200c435` |
| `transition-ease-in-out-quarter.png` | 1.25 s | 640x360 | `344986dd1a90d39fa1df9dc050b4ef6bea76b3014cf96417760f429956df86c4` |
| `nested-composition-two-level.png` | 0.5 s | 640x360 | `bcde44b0f8afd2540bb100aa4b753240be22e1eb0aca614c66a6383820c6b65a` |
| `nested-composition-two-level-after-edit.png` | 0.5 s | 640x360 | `a6ed84b5179676705b9c6302b432b06460a4d9b7b6927b569efecaa5826d568e` |
The latest fixture revision additionally writes `generated-basic-shapes.png` (640x360, 0.5 s), with a filled cyan rectangle, yellow ellipse, and white horizontal line on a dark source. The line is centered over the ellipse; the transparent exterior still shows the base color. SHA-256: `a686737ef05ddca67453aee4cfc12f398bba6033618bd4cb589df5bece9464b8`.

The real FFmpeg integration fixture also renders a 440 Hz WAV through the production playback-audio graph at unity and 0.5 audio-track gain. It decodes the resulting float PCM and verifies the half-gain RMS is within 0.02 of half the unity RMS. This is an audio assertion rather than a saved image, so no redundant media artifact is retained.

The new transition integration frame SHA-256 is `FFC18F33CD78A266D5CAD98A4D970ED9DE24F3EF971E6E01048751D137D6D396`.

## Proxy acceptance frames

The same real FFmpeg integration fixture also creates a 1280x720 standalone test video and generates a video-only 960x540 H.264 proxy. It saves and reopens a 640x360 composition with proxy preview enabled, then renders original and proxy preview frames at 0.5 s. Their decoded pixels differ (the proxy is lossy), while the content geometry and source-coordinate framing match. The fixture renders two final MP4s with preview proxy mode enabled and disabled, decodes the frame at 0.5 s, and asserts the resulting RGBA pixels are identical. It also inspects the generated FFmpeg input arguments: preview uses the attached proxy and final export uses the original file.

I inspected the original-preview, generated-proxy-preview, and final-export frames. All three show the same full-frame test pattern without clipping or framing shifts; proxy compression produces small pixel differences, while toggling proxy preview does not affect final export pixels. This validates proxy graph selection and export fidelity, not GUI interaction or performance improvement on a physical editing workload.

| File | State | Resolution | SHA-256 |
|---|---|---:|---|
| `proxy-preview-original.png` | original media, 0.5 s | 640x360 | `B211ACB653616C2ED017DB2FF270B86C9D17E9DFCB292DC5A6168B3A5D0F9055` |
| `proxy-preview-generated.png` | generated proxy media, 0.5 s | 640x360 | `B06D6A333927149A6F9942967C86BAF3D71AB6921A3407F4E490B4A265A4A3D0` |
| `proxy-export-uses-original.png` | final MP4 export with proxy preview enabled, 0.5 s | 640x360 | `15D8B38211B7D453859915D81B9721E8200F5A72668B72586999E867F05C18E3` |
