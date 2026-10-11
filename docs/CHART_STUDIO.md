# Sono Chart Studio

Chart Studio is a generic LevelData authoring surface. It edits
`formats::LevelData` directly and previews by sending a snapshot through
`RenderConfig::render_frame`, which reaches the same Watch runtime and render
pipeline used elsewhere in the application. It does not keep a second chart
graph or runtime.

## Create and open

Open **Chart Studio** from the main workspace. **New LevelData** starts with an
empty `entities` list and no `bgmOffset`; this is a neutral document, not a
claim that every engine accepts an empty level. **Open LevelData** accepts the
same standalone JSON/gzip and legacy level package forms as the current loader.
The editor applies a 128 MiB compressed and decompressed input limit to protect
the desktop process.

Choose the engine in **Project / inputs**, then choose **Inspect selected
engine** in Chart Studio. The palette is populated from the selected package's
`EnginePlayData` level-archetype declarations. `EngineWatchData` callback
archetypes are a separate runtime surface and are not treated as proof that an
entity type is valid for level authoring. If PlayData declarations are absent,
the palette reports that limitation while raw archetype IDs remain editable.
The editor never assigns note types or other gameplay meaning based on a name
or visual behavior.

When no current engine/schema match is available, the UI explicitly enters
**Generic / Raw Authoring Mode**. It provides a structured inspector for
primitive values in arbitrary nested JSON data, alongside the complete raw
entity JSON editor for adding/removing fields and editing empty containers.
The inspector exposes data paths and JSON types only; names such as `x`, `beat`,
`lane`, or `ref` are not assigned gameplay meaning. Numeric values, including
negative values, are editable. Extremely large integers and overlarge/deep
payloads remain available through raw JSON rather than risking precision loss
or excessive UI work.

The entity list supports individual and Ctrl/Command multi-selection, create,
duplicate, delete, reorder, and copy/cut/paste. The in-app copy buffer
preserves entity JSON exactly. References are not renumbered because generic
LevelData does not give this editor universal identity/reference semantics.
Property edits currently apply to one entity at a time. Each committed
operation is undoable/redoable through a bounded 256-operation edit log.

## Authoring schemas

An optional JSON schema can supply labels, categories, descriptions, explicit
timing bindings, bounded property descriptors, and inert `initial_data`
templates for new entities. Required-field, numeric-range, and enum
constraints are authoring conventions; they do not establish Sonolus runtime
requirements. The schema must match the selected engine's semantic content
SHA-256 and exact PlayData archetype names. That identity includes metadata,
configuration, ROM, PlayData, and Watch data, but excludes the local archive
path and ZIP container bytes. A stale or invalid schema falls back to raw
mode. The schema is inert data: it has no expression evaluator, filesystem
access, network, or process execution. Files are limited to 1 MiB, templates
to 64 KiB, JSON node/depth counts are bounded, archetype/property counts are
bounded, and JSON Pointers are limited in size and depth.

Use the installed CLI to obtain the identity. A schema has this shape (replace
the placeholder identity and archetype with values from that engine):

```powershell
$renderer = Join-Path $env:LOCALAPPDATA 'Programs\Sono-Renderer\Sono-Renderer.exe'
& $renderer engine-identity 'engine.zip'
```

```json
{
  "format": "sono-chart-authoring-schema",
  "schema_version": 1,
  "engine": { "content_sha256": "<64 hex characters from engine-identity>" },
  "archetypes": [
    {
      "id": "<exact EnginePlayData level-archetype name>",
      "label": "<human-readable label>",
      "category": "<optional group>",
      "initial_data": [{ "name": "time", "value": 0.0 }],
      "timing": { "path": "/0/value", "unit": "watchSeconds" },
      "properties": [
        {
          "path": "/0/value",
          "label": "Time",
          "description": "A schema-declared authoring field",
          "required": true,
          "minimum": 0,
          "maximum": 100
        }
      ]
    }
  ]
}
```

The current UI uses schema labels/categories as palette and field labels,
creates described archetypes from `initial_data`, provides typed controls for
existing described values, and reports schema constraint findings separately
from structural LevelData checks. Edits remain a draft until **Apply raw
JSON**. Paths are JSON Pointers into the entity's `data` value, so `/0/value`
means the first data entry's `value`; there is no extra `/data` prefix. Unknown
fields remain editable in the raw data inspector/editor. A wrong-engine
schema is rejected and raw mode stays available.

Schema version 1 remains backward compatible: `initial_data` and `required`
are optional additions, so schemas written before those fields continue to
load. Unknown schema fields are retained by the schema model. An example
template is in `examples/chart-authoring-schema.json`; replace its identity
and archetype with values from the target engine before loading it.

## Timing, validation, and preview

`bgmOffset` is editable as LevelData seconds. Entity timing remains
engine-defined. A matching schema may explicitly declare a JSON Pointer as
`watchSeconds` or `mediaSeconds`; only those entities appear in the timeline.
Media seconds are mapped to Watch seconds using the existing invariant
`media time = Watch time + bgmOffset`. Other fields stay raw unless a schema
property descriptor provides an applicable typed control. The existing Watch
host continues to own `#BPM_CHANGE`, `#TIMESCALE_CHANGE`, and beat/time
conversion; Chart Studio does not add a competing BPM_CHANGE parser. A
separately accepted authoring reference grid can be stored in the existing
render-project extension; it does not enter LevelData or the Watch runtime.
The preview is a still frame at an absolute Watch time, not continuous
gameplay playback. It captures a canonical LevelData snapshot into a temporary
gzip file, then calls the ordinary renderer. A changed document invalidates
the displayed revision; a background result from an older revision is
discarded.

**Validate structure** checks the document shape and `bgmOffset`. Matching
schema constraints are reported as authoring conventions, separately from
structural errors; neither proves engine runtime acceptance or reference
semantics. The existing
Compatibility panel and Watch runtime remain authoritative for those checks.
See [tempo analysis and reference grids](CHART_STUDIO_TEMPO_ANALYSIS.md) for
local audio audition, waveform display, BPM proposals, and the explicitly
applied authoring grid.

## Save project vs export

**Save in render project** stores the current canonical LevelData JSON under a
forward-compatible extension key in the existing `.sonoproject` file. The
project remains the input/configuration container; profile settings are not
copied into chart data. The existing project size limit still applies.

**Export LevelData** writes a standalone `.json.gz`. It uses canonical
`formats::LevelData`, deterministic gzip metadata, a same-directory temporary
file, file sync, production-parser round-trip validation, and atomic
publication. Existing files require an explicit overwrite action. The result
does not depend on Chart Studio metadata or a private store and can be passed
to the ordinary renderer.

CLI equivalents (PowerShell, using the installed executable):

```powershell
$renderer = Join-Path $env:LOCALAPPDATA 'Programs\Sono-Renderer\Sono-Renderer.exe'
& $renderer engine-identity 'engine.zip'
& $renderer validate-level-data 'level.json.gz' --engine 'engine.zip' --json
& $renderer export-level-data 'level.json.gz' 'output.json.gz'
& $renderer export-level-data 'level.json.gz' 'output.json.gz' --overwrite
& $renderer run-watch 'engine.zip' 'output.json.gz' --time 0
```

## Current limits

This pass supplies generic authoring and an actual Watch-backed still-frame
preview. It does not implement a chart spatial canvas, schema-defined placement
tools, automatic entity snapping/quantization, an engine-independent
BPM_CHANGE map, time-shifted graph-aware paste, bulk property editing,
reference graph repair, autosave recovery, Presentation Studio linking, or
full compatibility diagnostics on unsaved state. Raw editing and the ordinary
compatibility/renderer workflow remain available for arbitrary engines.
Interactive GUI use should be manually exercised on the target platform;
automated/model tests do not establish visual usability.
