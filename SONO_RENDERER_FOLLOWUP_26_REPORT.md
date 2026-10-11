# Sono-Renderer Follow-up #26 — Chart Studio report

## Summary

Implemented a generic LevelData editing and export path around the existing
`formats::LevelData` and Watch runtime. Chart Studio is a new main-workspace
tab. It opens bounded JSON/gzip/legacy level inputs, discovers archetypes from
the selected engine, edits entities, supports undo/redo, saves the editable
snapshot in the existing render project, previews a snapshot through the real
renderer, validates basic structure, and exports deterministic standalone
`.json.gz` LevelData.

This is a useful generic authoring core, not the complete timeline/audio/visual
chart editor described in the prompt. The explicit deferrals below are
intentional; no engine gameplay taxonomy is guessed to make the editor appear
more complete.

## Architecture and acceptance items

1. **Existing architecture — inspected.** The repository has one
   `formats::LevelData`, `formats::EnginePackage`, generic Watch data/runtime,
   `RenderConfig::render_frame`, a versioned `RenderProject`, and an existing
   project/GUI/CLI boundary. It had no chart editor, shared edit-command stack,
   authoring schema, chart timeline, or audio playback/waveform subsystem.
2. **LevelData representation — inspected.** `LevelData` contains optional
   `bgmOffset`, entity list, optional metadata and flattened unknown root
   fields. `LevelEntity` keeps arbitrary archetype/data JSON and flattened
   unknown entity fields.
3. **Canonical editing — implemented.** `ChartDocument` edits the existing
   `formats::LevelData` and `LevelEntity` types; it adds no second chart graph.
4. **Chart Studio architecture — implemented.** The GUI delegates data edits,
   validation and export to `chart_studio`; preview calls the production
   `RenderConfig::render_frame` path with a temporary LevelData snapshot.
5. **Generic entity editor — implemented.** Create, inspect, raw JSON edit,
   duplicate, delete, move, copy/cut/paste and multi-select are available.
6. **Archetype discovery — implemented.** Names come from the selected
   engine's Watch archetype declarations. No TAP/FLICK/HOLD palette is
   hardcoded.
7. **Inspector — implemented.** A selected entity shows its raw archetype and
   complete JSON. A matching schema may add property controls.
8. **Raw editing — implemented.** A draft is parsed before commit; invalid JSON
   leaves canonical LevelData unchanged. Flattened unknown entity fields are
   retained.
9. **References — partially implemented.** Copy/paste preserves reference
   values exactly. Identity remapping, reference discovery/repair and graph
   validation are deferred because generic LevelData does not define one
   universal reference convention.
10. **Selection — implemented.** Single selection and checkbox/Ctrl/Command
    multi-selection are available. Box selection and semantic selection
    filters are deferred.
11. **Copy/paste/duplication — implemented, generic semantics.** The in-app
    copy buffer preserves selected entity JSON. Duplicates are inserted next
    to their source. Reference IDs are deliberately not rewritten.
12. **Undo/redo — implemented.** Insert, delete, replace, reorder and
    `bgmOffset` edits use an edit-operation log capped at 256 commands. No
    `ChartUndoManager2` or full-level snapshot per command was introduced.
13. **Timing model — partial.** `bgmOffset` is an undoable LevelData edit.
    Entity timing remains engine-defined raw data unless an authoring schema
    declares its JSON Pointer and whether it is Watch seconds or media
    seconds. Media-to-Watch seeking uses `media time = Watch time + bgmOffset`.
14. **BPM/grid — deferred.** Existing Watch owns `#BPM_CHANGE`,
    `#TIMESCALE_CHANGE` and beat conversion. Chart Studio does not duplicate
    that parser or offer a beat grid yet.
15. **Quantization — deferred.** No universal beat grid is inferred.
16. **Snapping — deferred.** No schema-defined spatial/lane constraints exist
    in this implementation.
17. **Timeline — partial.** A virtualized list is shown only for explicit
    schema-declared Watch/media-second fields; it sorts by the declared value
    and can select/seek to an entity. It does not edit a universal time field.
18. **Spatial/chart view — deferred.** No gameplay canvas or placement tools
    were added.
19. **Authoring schema — implemented, first version.** Versioned JSON can
    provide archetype labels/categories, JSON Pointer property descriptors,
    bounds/enums and explicit timing descriptors.
20. **Schema identity/version — implemented.** Schema v1 binds to SHA-256 of
    loaded engine metadata/configuration/ROM/Watch contents, not title or file
    name. `renderer engine-identity <engine>` prints this content identity.
21. **Schema safety — implemented.** 1 MiB schema limit; 4096 archetype and
    16384 property caps; JSON Pointer length/depth/escape checks; bounded
    labels/enums; finite ordered numeric bounds. There is no expression
    evaluator, filesystem/network access or process execution.
22. **Schema-defined tools — partial.** Matching labels, descriptions,
    categories, numeric bounds, enums and simple type-aware controls work.
    Placement/connection/grouping tools, defaults and visual representations
    are deferred.
23. **Reference schema — not supplied.** Tests exercise a schema bound to a
    synthetic generic engine. No real bundled engine schema is included because
    the repository does not provide authoritative authoring metadata for a
    generic reference engine. A documented template and engine identity CLI
    support external schema authoring.
24. **Generic fallback — implemented and tested.** Engines without schemas
    still expose raw archetype IDs and arbitrary entity JSON editing.
25. **Live Watch preview — implemented.** A background worker writes an
    immutable temporary gzip snapshot and calls the regular renderer at an
    absolute Watch timestamp.
26. **Preview invalidation — implemented.** Each chart edit/undo/redo advances
    a revision. A result from an older revision is discarded and the displayed
    frame is marked stale.
27. **Audio/waveform — deferred.** This checkout has export audio processing,
    but no chart-editor playback transport or waveform generator.
28. **Validation — partial.** Structural checks cover finite `bgmOffset`,
    string/nonempty archetype IDs and warnings for undeclared engine
    archetypes. This is not full engine property/reference validation.
29. **Compatibility/diagnostics — reused, not embedded.** Existing
    Compatibility and Watch runtime remain authoritative. Unsaved-chart
    compatibility analysis and a structured chart diagnostic panel are
    deferred.
30. **Project integration — implemented.** The existing RenderProject stores
    a versioned canonical LevelData snapshot under a flattened extension key.
    Normal project Save includes an active Chart Studio document. Profiles
    remain separate.
31. **Autosave/recovery — deferred.** No chart crash-recovery journal was
    added.
32. **Save vs Export — implemented.** Project Save persists editable state in
    the existing project. Export produces an independent Sonolus LevelData
    artifact.
33. **LevelData export — implemented.** `chart_studio::export_level_gzip`
    serializes the canonical model and emits `.json.gz`.
34. **JSON serialization — implemented.** Unknown root/entity/data values
    survive model serialization. Package `item.json` metadata attached by the
    legacy adapter is stripped when exporting a package, because it is not
    standalone LevelData.
35. **Gzip — implemented and tested.** Gzip uses a fixed mtime and the output
    is decoded/parsed through the production LevelData parser before publish.
36. **Atomic export — implemented.** Same-directory temporary file, flush and
    file sync precede atomic publish. Existing output requires explicit
    overwrite; no-clobber publish is used otherwise.
37. **Export validation — implemented.** Structural validation plus exact
    canonical round-trip through `formats::load_level_data` occurs before the
    destination is replaced.
38. **Determinism — tested.** Two exports of the same fixture produced the
    same gzip SHA-256. Example: `ef36debb4bb9fe6c3c0ac8b51336a1ce0aad618b6f0d8494f9805aac1290de78`.
39. **CLI — implemented and exercised.** `engine-identity`,
    `validate-level-data [--engine ...] [--json]`, and
    `export-level-data [--overwrite]` were added. Existing `run-watch` and
    `render-video` were used unchanged.
40. **Presentation Studio linking — deferred.** The exported level can be
    loaded by normal renderer workflows. There is no in-memory Presentation
    Studio handoff.
41. **Onboarding/help — implemented.** First-run welcome, settings search and
    offline help link to Chart Studio and explain generic/schema-rich support.
42. **Large-level behavior — partially addressed.** Entity and timeline rows
    use egui virtualized rows. UI import caps compressed and decompressed
    payloads at 128 MiB and limits ZIP member count. No dedicated large-level
    benchmark was run.
43. **Generic fallback test — implemented/tested.** Synthetic arbitrary
    archetype edits, duplicate/delete, undo/redo, and unknown field retention
    are covered.
44. **Rich-schema test — partial.** Synthetic engine identity match/mismatch
    and JSON Pointer validation are tested. No bundled real-engine rich schema
    exists to test human-authored gameplay semantics.
45. **Second engine — not run for Chart Studio.** The production render check
    below uses the supplied Next RUSH engine.
46. **Existing-level round-trip — tested.** The real 1,131-entity
    Baumkuchen fixture was exported and reloaded with canonical equality.
47. **Edited-level round-trip — partial.** Generic edit operations and export
    are tested on canonical model fixtures; the real engine file was round-trip
    exported unchanged. A real-engine edit followed by MP4 was not performed.
48. **Unknown data — tested.** Unknown root, entity and arbitrary nested JSON
    values survive export/re-import.
49. **Gzip validation — tested.** Header, decompressed JSON/LevelData, stable
    hash and overwrite refusal were checked.
50. **Normal renderer — tested.** The exported real fixture passed
    `run-watch` with Next RUSH/ProSeka resources and produced a 320x180 PPM
    frame. The ordinary `render-video` path also produced a 3-frame MP4.
51. **Real workflow — partial, production-path verified.** CLI export →
    validation → ordinary Watch → ordinary MP4 export succeeded. The GUI was
    not interactively driven, and the real fixture was not semantically edited
    in the GUI before rendering.
52. **Performance — measured only for the tiny validation render.** The
    3-frame, 320x180 MP4 run took 12.506 seconds in this environment; it is not
    a benchmark. Performance/diagnostic persistence emitted access-denied
    warnings for `%LOCALAPPDATA%\Sono-Renderer`, but rendering and FFprobe
    validation completed.
53. **Regression — tested.** Baseline `cargo test --lib`: 273 passed, 4
    ignored. After implementation, `cargo test`: 281 library + 24 CLI + 3
    arctan2 + 101 m0 + 9 Watch scheduler tests passed; 4 ignored. After the
    final small GUI/project edits, the 8 Chart Studio tests and the new CLI
    parser test passed again.
54. **GUI manual test — unavailable.** GUI code was formatted and
    build-checked; no interactive desktop automation was available. The
    generated PPM was converted to PNG and visually inspected, but that checks
    renderer output, not Chart Studio widget usability.
55. **Platform — Windows tested.** Commands, real-engine render and GUI build
    check ran on Windows. Linux/macOS builds and interactive use were not
    tested here.
56. **Known limits — explicit.** No chart spatial canvas, beat grid,
    quantization, snapping, graph-aware reference repair, time-offset paste,
    bulk property editing, autosave recovery, audio playback/waveforms,
    unsaved-state compatibility report, bundled reference schema or
    Presentation Studio handoff.

## Commands used

```powershell
cargo fmt --all
cargo check --all-targets
cargo test
cargo test --lib chart_studio::tests
cargo test chart_studio_identity_validation_and_export_commands_parse
cargo clippy --all-targets
```

The real fixture files and the tiny MP4/PPM validation outputs are under the
ignored `target/chart-studio-validation/` directory; no large binary fixture
was added to the repository. `cargo clippy` completed but reported the
repository's existing broad warning set (including a deprecated atomic API
and dead code); the new Chart Studio module had no Clippy findings in the
filtered pass.
