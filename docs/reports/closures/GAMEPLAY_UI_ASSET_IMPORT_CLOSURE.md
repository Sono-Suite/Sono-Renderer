# Gameplay UI Asset Import Closure

## Result

The existing Presentation Asset Pack can now become an editable Gameplay UI
profile through the native settings UI. The workflow reuses the manifest,
`RendererUiConfig`, bitmap glyphs, the production HUD compositor, existing
profile/project persistence, and the normal render command. It does not add a
second asset format or renderer.

## Implementation

- **Import and path handling:** `Import Asset Pack...` accepts the versioned
  `sono-renderer-presentation-asset-pack` manifest. The profile loader detects
  that exact format and offers import; unrelated JSON stays an error. Relative
  sources resolve under the manifest directory or a user-selected root.
  Canonicalized paths are confined to that root, including nested and Unicode
  paths. Import never edits or copies source files. Missing, invalid, and
  unsupported entries remain visible in the import report.
- **Profile conversion:** Supported images, sprites, and glyphs are registered
  as `pack://` assets in the existing Gameplay UI schema. Only referenced image
  bytes are decoded into the profile. Exact one-character glyph metadata is
  mapped automatically; duplicate or absent character labels stay unresolved.
  Judgment elements are generated only from explicit `visible-label:*` tags
  within a judgment artwork family. Other asset names and families do not
  become gameplay semantics by inference.
- **Editor workflow:** Gameplay UI settings show imported assets, pack status,
  missing/invalid references, glyph-family mappings, and unassigned families.
  Users can add score/combo/life counters and image elements, select their
  bindings and glyph families, edit visibility, position, size, scale, opacity,
  anchor, alignment, and layer order, then save a versioned profile. The preview
  is labeled as sample-only and uses 123456 / 789 / 880 / PERFECT; it does not
  write to Watch state. An empty profile is called out explicitly.
- **Repair and persistence:** Profiles retain their pack/root references and
  element configuration. Strict loading reports missing external files; the
  GUI can still load the saved layout for repair and relink it to a selected
  asset root. Profile save is atomic. Existing project/profile resolution
  preserves the imported references, glyph mappings, bindings, and layout.
- **Render integration:** Imported assets use the production Gameplay UI
  compositor, not a preview-only renderer. The exported MP4 test uses the
  production `render-config` command and a real engine/level/resource fixture.

## Validation evidence

Commands were run from the repository root with offline, locked dependencies.

| Validation | Result |
| --- | --- |
| `cargo test --locked --offline --lib asset_pack -- --test-threads=1` | Passed, 2 tests. |
| `cargo test --locked --offline --lib explicit_asset_root_supports_unicode_nested_paths_and_remains_confined -- --test-threads=1` | Passed. Covers nested/Unicode source paths and root confinement. |
| `cargo test --locked --offline --lib missing_pack_assets_keep_profile_editable_and_can_be_relinked_to_a_new_root -- --nocapture --test-threads=1` | Passed. Missing file is reported by strict load; saved element configuration survives repair load and resolves under the relocated root. |
| `cargo test --locked --offline --lib supplied_reference_profile_uses_the_versioned_asset_registry -- --test-threads=1` | Passed. Existing profile compatibility retained. |
| `cargo test --locked --offline --lib supplied_pack_import_renders_its_png_and_survives_profile_and_project_reopen -- --nocapture --test-threads=1` | Passed. Supplied pack PNGs rendered; standalone profile and project/profile round-trips retained the layout and references. |
| `cargo test --locked --offline --lib gameplay_ui_event_state_is_composited_by_real_preview_pipeline -- --nocapture --test-threads=1` | Passed. Imported artwork was composited against the same prepared gameplay frame through CPU and WGPU paths; gameplay event state matched and image differences stayed within the test tolerance. |
| `cargo test --locked --offline --test gameplay_ui_asset_import_export -- --ignored --nocapture --test-threads=1` | Passed. Real fixture render produced H.264 MP4, 640×360, 12 fps, 3 seconds. FFprobe and extracted-frame assertions passed. |
| `cargo check --locked --offline --all-targets -j 2` | Passed, including GUI and integration-test targets. |
| `cargo fmt --all -- --check` | Passed. |
| `git diff --check` | Passed; Git printed existing line-ending normalization notices only. |

The real-pack export artifacts are retained under the ignored `target/`
directory:

- MP4: `target/gameplay-ui-import-acceptance/imported-assets-1791553877554.mp4`
- Extracted frame: `target/gameplay-ui-import-acceptance/imported-assets-1791553877554-frame.png`
- Saved UI profile used by the export: `target/gameplay-ui-import-acceptance/imported-ui-1791553877554.json`

I inspected the extracted frame. It visibly contains the imported score
digits, imported life digits, the pack's tagged PERFECT sprite, the gameplay
image, and the protected watermark. The test also checks that a color sampled
from the source score glyph appears in the exported frame, guarding against a
built-in text fallback masquerading as imported artwork.

## Limits and manual check

- The supplied combo glyph family has no explicit character labels and its
  manifest marks its prefix semantics unverified. It is intentionally not
  auto-mapped. The editor exposes per-digit asset selection so the user can
  make that assignment deliberately; this manual GUI interaction was not
  exercised in this environment.
- Video entries remain catalogued but are unsupported by the Gameplay UI HUD
  compositor. External pack assets are referenced rather than copied into the
  project, so a portable project must keep those files available or repair the
  asset root after moving them.
- Native GUI behavior was build-checked, not interactively exercised. A
  packaged installer was not tested.
- The export acceptance used the real Larp 64x fixture and confirmed score,
  life, and judgment artwork. Combo is available through the UI but was not
  included in that exported-frame acceptance because its supplied glyph
  semantics require a deliberate assignment.

Manual GUI checklist:

1. Open **Gameplay UI** and click **Import Asset Pack...**; select
   `presentation-pack.json`.
2. If files are missing, select the directory that contains the manifest's
   relative asset paths. Confirm the missing/invalid summary clears for the
   repaired files.
3. Add Score, Combo, and Life counters. Select the score/life bitmap families;
   explicitly map each combo digit if that family is desired.
4. Confirm judgment elements were created only from tagged judgment artwork.
   Adjust positions and order, then inspect the sample-only preview.
5. Save the UI profile, enable Gameplay UI in a render configuration/project,
   reopen it, and export. Confirm the chosen pack artwork appears in the MP4.

The worktree already contained many unrelated modified and untracked roadmap
files. They were preserved; no cleanup or reset was performed.
