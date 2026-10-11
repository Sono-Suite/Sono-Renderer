# Gameplay HUD asset inventory

`assets/` contains 199 media files: 195 PNG images and four MP4 clips, plus the versioned `presentation-pack.json` manifest. I inventoried the directory tree and inspected the score, rank, life, combo, judgement, control, background, lane, and video groups. The default profile uses the original PNG artwork for HUD components wherever the source supplies it. The UI renderer remains generic: asset IDs, layout, clipping, visibility, and state-to-style rules live in JSON. See [`docs/PRESENTATION_STUDIO_ASSETS.md`](../docs/PRESENTATION_STUDIO_ASSETS.md) for Follow-up #23's complete inventory, structural classifications, and known gaps.

## Assets used by `default.json`

| HUD part | Assets | Profile composition |
|---|---|---|
| Score panel and gauge | `score/bg.png`, `score/bar.png`, `score/fg.png` | `bg` supplies the striped frame, SCORE label, dark track, and rank badge backing. The `bar` image is clipped to the score fraction. `fg` draws the C/B/A/S marker layer over it. All three retain the same 2069x446 canvas bounds and aspect ratio. |
| Score digits and delta | `score/digit/{0..9}.png`, `score/digit/s{0..9}.png`, `score/digit/+.png`, `score/digit/s+.png` | Per-character image fonts render the white score digits and their supplied dark shadows; the bonus delta uses its own plus sprites and the same digit art. |
| Rank badge and caption | `score/rank/chr/{a,b,c,d,s}.png`, `score/rank/txt/en/{a,b,c,d,s}.png` | The rank glyph and grade-colored SCORE RANK caption are selected from the externally supplied `rank_code`. The caption PNGs are already rendered text artwork with matching alpha masks; the profile now uses those instead of generic text. The renderer does not derive a grade from score. |
| Life panel and gauge | `life/bg.png`, `life/normal.png`, `life/danger.png`, `life/mask.png`, `life/overflow.png` | The panel contains LIFE, heart, track, and pause artwork. The normal fill is clipped by the mask and reveal fraction, switches to danger art below 300, and uses overflow art above 1000. The source region and gauge aspect ratio are specified in the profile. |
| Life digits | `life/digit/{0..9}.png`, `life/digit/s{0..9}.png` | Per-character foreground and shadow images display the live value. The profile uses tighter horizontal advance to match the original digit spacing. |
| Combo title and digits | `combo/pe.png`, `combo/pt.png`, `combo/p{0..9}.png`, `combo/b{0..9}.png` | `pe` supplies the soft title glow, `pt` the crisp COMBO title, `p` the reference's purple-to-cyan digits, and `b` the transient burst layer for the latest special-note state. The burst is layered behind the colored digit sprites. |
| Judgement | `judge/{1..6}.png` | Profile visibility maps PERFECT, GREAT, GOOD, BAD, MISS, and AUTO state strings to the supplied image sprites. The sprite ratio is preserved. |

## Relevant assets intentionally not placed by this profile

- `score/rank/txt/jp/{a,b,c,d,s}.png` contains the same grade-specific caption treatment in Japanese. The supplied reference is English, so `txt/en` is used.
- `score/digit/%`, `-`, `.`, `n`, and the corresponding shadow symbols are available, but this profile has no accuracy provider, negative score delta, or decimal readout to bind them to. The score delta currently exercises `+` and numeric digits.
- The alternate combo families (`n*`, `p*` underscore variants, `b*` underscore variants, `n_c`, `b_c`, `nt`, and related special glyphs) represent other palettes, outlined/alternate digits, or achievement/tag displays. This reference uses the normal `p` palette plus the transient `b` burst. `combo/p_c.png` is a combined title/glow sprite; it is not layered because the source composition uses separate `pe` glow and `pt` title assets, and combining it with `pt` would duplicate the letters. The root `combo.png` is a static full-canvas title placement; separate elements keep the title anchorable and profile-responsive.
- `life/digit/n.png` and `sn.png` are blank/leading-zero helper glyphs. This profile displays the ordinary numeric life value without leading spaces.
- `life.png` and `life-v1.png` are flattened HUD snapshots with a fixed life value/bar and pause icon. The split `life/` assets are used so live life state controls the digits and bar instead of painting stale values over them.
- `autolive-en.png` is available, but the real Watch scene in the validation frames already draws an `AUTO` control at the lower right. The profile leaves it there to avoid a second, offset control; the captured Watch version is dimmer and slightly left of the video reference. Pause is already part of `life/bg.png` and is not redrawn separately.
- `score/rank/txt/*` textures and standalone non-HUD menu/difficulty artwork are not used as generic rectangles. The rank caption images are used directly; menu artwork does not belong in this in-game HUD. The AP MP4 clips are video content, and the generic profile API has no video layer.
- `background_full.png`, `background-v1_full.png`, `lane*.png`, difficulty backgrounds, and chart cover art belong to the app/chart scene rather than this HUD profile. The validation PNGs keep the scene produced by the actual Watch renderer underneath the profile instead of substituting an unrelated static background.

## Reference composition inspected

`TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2` composes the corresponding layers: life background and masked normal/danger/overflow fills; score background, clipped bar, rank text texture, rank glyph, and foreground markers; combo `pe`/`pt` title with `p` digits and a temporary `b` burst; and judgement sprites. The 16:9 and 4:3 `main2_*.object` layouts supply screen-relative placements. The profile follows those asset roles and the supplied screenshot's visible placements while keeping the mechanics in generic renderer features and profile data. The external source is a visual/behavior reference; its implementation is not embedded in the renderer.
