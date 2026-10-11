# Follow-up #23: presentation asset inventory

This is an inventory of the current repository's `assets/` directory, not a
claim that a full presentation editor exists. The manifest is consumed by the
existing gameplay UI profile loader and by the asset inspection command.

## Inventory

The directory contains 199 media files (195 PNG and 4 MP4) plus the JSON
manifest. The manifest describes 197 logical entries and two 4:3 video
variants. All 199 media paths are represented. PNG dimensions, format alpha
capability, byte counts, SHA-256 identities, and video stream metadata are
recorded in `artifacts/presentation-studio/asset-pack-inspection.json`.

| Manifest kind | Entries | Current use |
|---|---:|---|
| Image | 51 | HUD panels, backgrounds, lanes, indicators, and other still artwork |
| Sprite | 12 | Layered UI/stage artwork classified as sprites; some composition meaning remains uncertain |
| Glyph | 132 | Life, score, and combo image glyph families |
| Video | 2 + 2 variants | Two H.264/AAC clip families, each with 16:9 and 4:3 media |

The inspection pass decoded all PNGs to establish dimensions and alpha
capability. It visually inspected contact sheets for numeric and combo glyph
families and representative frames from every supplied AP clip. The AP clips
show an “ALL PERFECT!!” celebration. The manifest therefore uses the
content-verified label `all-perfect-animation`; it does not infer gameplay
state or trigger semantics from that content.

## Verified reference-template roles

`ui-profiles/default.json` references the supplied images by stable
`pack://<asset-id>` IDs. The compositor remains generic; asset selection,
element geometry, rank thresholds, masking, state bindings, and visibility are
profile data.

| Presentation part | Pack entries | Composition in `default.json` |
|---|---|---|
| Score panel/gauge | `score.bg`, `score.bar`, `score.fg` | Panel background, clipped progress strip, and marker foreground |
| Score glyphs | `score.digit.*` | Numeric glyph mapping, plus sign, and shadowed delta |
| Rank | `score.rank.chr.*`, `score.rank.txt.en.*` | Rank image and supplied English caption, selected from external presentation state |
| Life panel/gauge | `life.bg`, `life.normal`, `life.danger`, `life.mask`, `life.overflow` | Panel, masked fill, threshold-based styles, and over-maximum fill |
| Life glyphs | `life.digit.*` | Per-glyph image text for life digits |
| Combo | `combo.pe`, `combo.pt`, `combo.p0..p9`, `combo.b0..b9` | Separate glow/title, colored number family, and burst layer |
| Judgement | `judge.1..6` | Image styles for PERFECT, GREAT, GOOD, BAD, MISS, and AUTO |

The following meanings are directly visible in the images or confirmed by
the reference composition in `TestingSuite/Sono-Overlay`: score/life panel
layers, the supplied rank captions and letters, the six judgement words,
English/Japanese language variants, and the `p` combo digits used by the
reference. The split life assets compose a live gauge; the flattened `life.png`
and `life-v1.png` are fixed-value HUD snapshots and are not used where values
must change.

## Preserved classifications and uncertainty

The `combo.n*`, `combo.p*`, and `combo.b*` families are separate selectable
glyph families. Their visible appearance and naming relationships are
recorded, but the prefixes are not treated as universal semantics. The default
template selects the `p` digits and uses `b` as the burst layer because that
matches the inspected reference composition. The `n` family and alternate
punctuation/underscore variants remain in the pack.

`score/digit/%`, `.`, `-`, and their shadow variants are preserved. The
reference profile maps the signs it currently renders; it does not invent
accuracy or decimal data bindings. `autolive-en.png` is not overlaid because
the real Watch scene already draws an AUTO control in the validation fixture;
overlaying it would duplicate the control. Tournament language variants are
ordinary image entries.

Background/lane generations, difficulty backgrounds, `master.png`, `over.png`,
`mv_line.png`, `rainbow.png`, `start_*`, `extra.png`, and `collab-tag.png` are
retained and structurally categorized. Where their exact gameplay meaning or
layer order cannot be established from repository references and visible
content, the manifest uses neutral family/tags rather than asserting a role.
The MP4 variants are identified by visible content, dimensions, frame rate,
codec, and audio stream; the generic UI profile has no video-layer primitive,
so they are inspectable assets but not yet composited by the presentation
profile.

## Asset-pack interface

`assets/presentation-pack.json` uses schema 1 of
`sono-renderer-presentation-asset-pack`. Entries have pack-local IDs, relative
source paths, structural types, optional family/language/tags, and declared
aspect-ratio variants. The loader preserves unknown entry fields, rejects
unknown schema versions, disallows traversal/absolute asset paths, confines
canonicalized sources to the pack directory, and enforces file, image, and
manifest size bounds. `pack://asset-id@4:3` selects a declared exact variant;
without a matching explicit variant it uses the base source.

Run `cargo run --offline -- inspect-presentation-asset-pack
assets/presentation-pack.json --json` to validate and inspect the pack. The
result is deterministic for unchanged files, apart from FFprobe metadata
formatting, and includes shared SHA-256 content identities. Profile rendering
loads only referenced images; full content hashing/metadata inspection is an
explicit operation.

## Importing a pack into a Gameplay UI profile

An asset pack is an artwork catalog; a Gameplay UI profile is the renderer's
editable arrangement of assets, glyph families, data bindings, and HUD
elements. In native renderer settings, open **Gameplay UI**, choose
**Import Asset Pack...**, and select `presentation-pack.json`. Import does not
rewrite the manifest or copy source images. Relative paths are resolved under
the selected asset root (by default, the manifest's directory), with canonical
path checks preventing traversal or symlink escape. If files are missing, the
import summary lists their IDs and paths and offers a different root.
**Load UI Profile...** still loads profiles; when the selected JSON has the
exact asset-pack format marker, it asks whether to import it. Unrelated JSON is
never treated as an asset pack.

The importer registers supported image, sprite, and glyph IDs using
`pack://<id>` references and keeps the pack/root references in the existing
version-1 Gameplay UI profile. Video and other non-image entries remain
catalogued as unsupported by the in-game HUD compositor. Glyph family names
and exact one-character mappings are retained. Duplicate characters are
reported as ambiguous and are not chosen automatically. Unlabeled glyphs,
including the supplied `combo.glyph-family.*` families, remain unassigned;
expand a family and explicitly select an image for digits 0–9 to build its
bitmap font. This preserves visual variants without guessing what `n`, `p`, or
`b` means.

The profile editor provides **Add Score Counter**, **Add Combo Counter**,
**Add Life Counter**, and **Add Image Element**. Select an element to edit its
enabled state, position, size, scale, opacity, anchor, z-order, value binding,
bitmap font, alignment, or image reference. Judgment sprites are configured
automatically only when the pack supplies a `visible-label:<value>` tag; the
condition reads the existing judgment field. Counter bindings select the
existing Score, Combo, or Life state sources and do not calculate gameplay
values. The design preview uses clearly labeled sample values solely for
layout; those values are never sent to Watch or to a render.

Use **Save UI Profile As...** to write a versioned profile atomically. Files
inside the profile directory are stored relatively when possible; otherwise
external pack and image paths remain external references. Load that profile in
**Gameplay UI**, enable the UI, and save it through the existing renderer
configuration/profile and `.sonoproject` workflow. Profile and project round
trips preserve the pack root, glyph mappings, bindings, element layout, and
visibility. If the external asset directory moves, choose **Select / repair
asset root...** and select the directory containing the manifest's relative
source paths. Missing inputs are not substituted with unrelated artwork.

Import results distinguish discovered entries, successfully loaded image
assets, missing files, invalid images, unsupported media kinds, registered
glyph families, unresolved/unassigned glyphs, and configured elements. A pack
with no recognized semantic tags may import successfully while still
requiring the creator to add and bind HUD elements; importing artwork alone
never implies a visible or gameplay-bound HUD.

## Current limits

The pack-to-profile workflow above targets the in-game Gameplay UI compositor.
It does not import these files as Presentation Studio media-bin items or NLE
layers; those remain separate systems. See
[Presentation Studio](PRESENTATION_STUDIO.md) for the standalone video-editing
workflow.
