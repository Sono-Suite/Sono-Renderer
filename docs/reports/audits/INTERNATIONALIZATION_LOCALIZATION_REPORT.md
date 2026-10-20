# Internationalization / Localization Report

**Status: PARTIAL. Prompt #43 is not complete.** The repository now has one
embedded catalog system, ten initial language catalogs, two developer test
locales, saved language selection, and primary-workflow wiring. Most of the
application UI remains English, and no real UI or packaged artifact was
visually exercised in this environment. The catalog set is complete only for
its current 73 message IDs, not for every application-controlled string.

## 1. Pre-change audit

The current application already had stable command IDs, workflow/workspace
preferences, and separate render, Chart Studio, and Presentation Studio data
models. It had no app-wide locale preference or localization service. User
facing strings were mostly Rust literals in the large GUI implementation;
command names/descriptions and diagnostics also contained English text.
Chart/engine metadata has its own data display behavior and is not application
localization. CLI and machine-readable formats were already separate from the
main GUI.

This pass did not replace those workflows or data models. The audit was source
inspection, not a complete extraction of every string; the latter remains a
substantial implementation gap.

## 2. Architecture and resources

`locales/registry.json` is the locale metadata source. Each locale has a
separate `messages.json`; English is the canonical key set. `build.rs`
discovers the catalog files and emits `include_str!` resources into Cargo's
generated output. The executable therefore does not need a checkout, a
working-directory-relative catalog path, or a runtime translation service.
`src/localization.rs` parses catalogs once through `OnceLock`, resolves the
selected catalog and English fallback, substitutes named parameters, and
keeps technical IDs out of display text. Plural lookup supports the current
locale set's one/few/many/other categories (including Russian and invariant
Japanese/Chinese/Korean forms); no production UI message currently uses a
plural key, and named-parameter substitution is presently exercised by tests
rather than a translated live counter.

Runtime loading skips an invalid individual catalog and falls back to English.
An invalid registry or unavailable English catalog falls back to the embedded
English source where possible, then to a minimal application-name catalog.
Strict catalog validation is separate from this resilient runtime path.
Catalogs and message values have size limits; message strings are data and do
not execute markup, scripts, commands, or file operations.

Resource layout:

- `locales/registry.json`
- `locales/en/`, `ja/`, `zh-CN/`, `zh-TW/`, `ko/`, `es/`, `pt-BR/`, `fr/`,
  `de/`, and `ru/`, each with `messages.json`
- `locales/test/pseudo/messages.json`
- `locales/test/rtl/messages.json`
- `locales/contexts.json` and `locales/README.md`

The resource copies are packaged for inventory and contributor workflow; the
running application uses the embedded catalogs. Editing a catalog therefore
requires rebuilding the application to see the edit.

## 3. Locale registry and translation status

The registry has ten release locales and two developer-only locales. Native
names are present in the selector. `zh-CN` and `zh-TW` have independent files
and are not produced by character conversion. Brazilian Portuguese is
identified as `pt-BR`, not generic Portuguese.

All ten release catalogs currently contain the same 73 canonical message IDs
as English. These messages cover the language selector, Home goal cards,
primary workflow navigation, Quick Render headings, selected Chart Studio
labels, selected Presentation Studio labels, and a few command labels. The
coverage status is explicitly `core-workflow`; it is not full-app coverage.

The non-English strings are machine-drafted and have not received human
linguistic review. JSON/key/placeholder validation does not establish
idiomatic wording, terminology consistency, or regional correctness.

## 4. Language selection, onboarding, and live switching

The Home screen offers a native-name language selector before a user enters a
workflow. Settings exposes the same selector. It is available during first
run and does not require completing onboarding. The selected ID is saved in
the existing workspace-preferences store (schema version 3); invalid IDs
fall back to English. The default is English; OS language auto-detection is
not implemented.

Catalog lookup reads the current locale on each UI frame, so translated
strings switch without restarting. UI font fallbacks are refreshed when the
locale changes. This logic is build-checked and persistence-tested, but the
English → Japanese → German → Korean → English sequence has not been
interactively exercised in the running GUI. Existing workflow and custom
workspace data use stable enum/command IDs and are not rewritten when locale
changes.

## 5. Command, workflow, and workspace integration

Workflow display labels resolve from stable `WorkflowId` keys. The global
navigation and command palette resolve command display names through the
existing stable command registry; localized command search also accepts the
canonical English label and stable ID. Command identity and dispatch remain
language-independent.

This is partial integration. Only a subset of command labels has catalog
entries. Command descriptions, categories, shortcut editor, shortcut help,
context menus, most toolbar/menu items, and disabled-action explanations are
still English. A localized description lookup helper exists but is not wired
through all command surfaces.

## 6. Chart Studio and Prompt #42 genericity

The Chart Studio title, introductory description, generic/raw-mode notice,
and a few actions use application message IDs. Canonical LevelData field
names, engine/archetype IDs, and raw authoring data are not translated or
modified. Unknown engine semantics are not machine-translated.

Most Chart Studio controls, inspectors, validation messages, and dialogs are
still English. Localized metadata from authoring schemas is not implemented.
Locale switching does not change schema identity or LevelData serialization.

## 7. Presentation Studio and user-created content

The Presentation Studio title, introductory description, and saved/modified
state use application messages. Project titles, media names, text layers,
captions, subtitles, imported SRT/VTT, and other creative content remain
project data and are not passed through the localization service. A UI locale
change does not set or rewrite composition fonts/styles.

Most timeline, inspector, effect, audio, transcription, microphone, and export
controls remain English. No end-to-end composition/render hash comparison was
run under multiple locales. The architecture keeps locale out of
`RenderConfig`, but a real-project all-locale render identity test remains
unverified.

## 8. Unicode and font handling

The eframe/egui UI receives up to two available host-provided CJK/Hangul font
fallback files, subject to file-size bounds. Windows candidate ordering gives
Japanese, Simplified Chinese, Traditional Chinese, and Korean locales a
locale-specific preferred system font; macOS/Linux use the listed system
fallback candidates. Fonts are not bundled, copied into projects, or used by
Presentation Studio's creative text renderer. No external font license is
introduced.

Catalog tests cover Unicode strings and locale metadata, and the GUI target
builds. Actual glyph coverage, TTC face selection, emoji rendering, mixed
script fallback, high-DPI layout, and visible clipping have not been verified
by screenshot or human inspection on the target operating systems. System
font availability varies; no font is guaranteed merely by the locale choice.

## 9. Errors, formatting, CLI, and generated reports

Most GUI errors, warnings, confirmations, validation findings, status text,
tooltips, accessibility labels, and third-party process context remain
English. There is no structured error-code-to-message catalog integration in
this pass. Original technical diagnostics are not translated.

CLI command names, flags, help, and human output remain English. Machine JSON,
project/schema fields, manifests, logs, cache identities, and diagnostic field
names remain unchanged. Locale-sensitive date/number formatting and localized
human-readable report prose are not implemented. CLI localization is
intentionally outside this GUI-first foundation, not a claim of support.

## 10. Pseudo-localization and RTL stress

`qps-ploc` expands/accentuates English messages and `qps-rtl` wraps test text
with RTL isolates and Arabic-script stress text. They are marked developer
locales and appear in the selector only in debug builds. Automated tests check
the transformation and release-selector exclusion.

This does not establish RTL layout support. The UI layout is not globally
mirrored; text direction metadata does not mirror the timeline, panels, or
dialogs. RTL is a text stress mode only, and domain time axes intentionally
remain left-to-right.

## 11. Persistence, performance, and security

The locale preference is stored in existing workspace preferences, not in
projects, profiles, RenderConfig, LevelData, manifests, history, or cache
keys. Tests cover migration from the previous preference schema, saving and
reopening a selected locale, and rejecting a tampered locale ID. Locale is
therefore application presentation state.

Catalog parsing is lazy and cached once per process. Locale switching does
not invalidate renderer, gameplay, media, cache, subtitle, or project state.
No performance benchmark was run. Font files are bounded by individual file
size and a two-font maximum; catalogs and registry are size bounded. Resource
paths are build-time resources, and runtime translations cannot execute code.

## 12. Validation tooling and translator workflow

Run the repository catalog validation from PowerShell:

```powershell
.\scripts\validate-locales.ps1
cargo test --lib localization::tests
```

The script validates UTF-8/JSON parsing, required locale IDs, fallback chains,
canonical key completeness, unknown keys, and placeholder parity. Rust tests
validate embedded catalog loading and reject duplicate message IDs. The
Windows release workflow runs the validator, and the package smoke test
validates locale copies in portable and installed staging directories.

`locales/README.md` documents locale metadata, message IDs, placeholders,
translator context, fallback, testing, and review status. Updating visible
translation edits requires a rebuild because catalogs are embedded.

## 13. Automated and manual tests

**Passed:** `cargo check --all-targets`; locale-resource validation reports
10 release locales, 2 developer locales, and 73 canonical messages;
`cargo test --lib localization::tests` passed four localization tests,
including strict embedded-catalog validation, placeholder parsing, duplicate
message IDs, fallback, Unicode/native names, plural-category selection,
pseudo/RTL behavior, independent Chinese catalogs, and locale-independent
RenderConfig serialization. `cargo test --lib workspace::tests` passed all
nine workspace tests, including locale preference migration/persistence,
tampered-ID fallback, localized Unicode command search, and stable command
IDs. The full library suite completed with **416 passed, 0 failed, 8 ignored**
in 253.63 seconds; it included real Chart Studio engine fixtures and the
existing Watch/GPU/video-export tests. The ignored tests retain their existing
external FFmpeg/fixture requirements.

Clippy, a release build, and a dedicated ten-locale output-hash comparison
were not run for this change.

**Not performed:** manual UI acceptance in any locale, screenshot inspection,
screen-reader/accessibility testing, OS font rendering validation, real
project render comparisons in ten locales, and running an actual packaged
installer/portable artifact. The current environment provides no interactive
desktop view through the available tools. The package smoke coverage is
configured in CI but has not been executed here.

## 14. Locale status matrix

“Catalog complete” means complete for the current 73 message IDs only. It
does not mean the whole application is translated. A technical catalog pass
is not a visual UI pass or linguistic review.

| Locale | ID | Catalog present | 73-key catalog | Technical catalog validation | Unicode / font validation | Major workflow UI | Package validation | Human review | Known issues | Status |
|---|---|---:|---:|---|---|---|---|---|---|---|
| English | `en` | PASS | PASS (canonical) | PASS | PARTIAL; no visual font test | PARTIAL; primary entry surfaces wired | UNVERIFIED artifact | Source | Most surfaces are English by design; no coverage audit completion | PARTIAL |
| Japanese | `ja` | PASS | PASS | PASS | PARTIAL; string tests only, system font unverified | PARTIAL; only selected entry surfaces | UNVERIFIED artifact | Machine-drafted; substantial GUI remains English | PARTIAL |
| Simplified Chinese | `zh-CN` | PASS | PASS | PASS | PARTIAL; string tests only | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; CJK glyph/layout unverified | PARTIAL |
| Traditional Chinese | `zh-TW` | PASS | PASS; independent file | PASS | PARTIAL; string tests only | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; CJK glyph/layout unverified | PARTIAL |
| Korean | `ko` | PASS | PASS | PASS | PARTIAL; string tests only | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; Hangul glyph/layout unverified | PARTIAL |
| Spanish | `es` | PASS | PASS | PASS | PARTIAL; no visual inspection | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; controls/dialogs/errors remain English | PARTIAL |
| Brazilian Portuguese | `pt-BR` | PASS | PASS | PASS | PARTIAL; no visual inspection | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; controls/dialogs/errors remain English | PARTIAL |
| French | `fr` | PASS | PASS | PASS | PARTIAL; expansion not visually tested | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; wrapping/layout unverified | PARTIAL |
| German | `de` | PASS | PASS | PASS | PARTIAL; expansion not visually tested | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; wrapping/layout unverified | PARTIAL |
| Russian | `ru` | PASS | PASS | PASS | PARTIAL; Cyrillic string only | PARTIAL | UNVERIFIED artifact | No | Machine-drafted; glyph/layout unverified | PARTIAL |
| Pseudo-localization | `qps-ploc` | PASS | N/A; generated | PASS; transformation test | UNVERIFIED visually | UNVERIFIED | UNVERIFIED artifact | N/A | Only catalog-backed strings transform; untranslated literals do not | PARTIAL |
| RTL stress | `qps-rtl` | PASS | N/A; generated | PASS; isolate test | UNVERIFIED visually | BLOCKED as full layout-direction test | UNVERIFIED artifact | N/A | Does not mirror egui layout | PARTIAL |

## 15. Exact supported claims and remaining gaps

Supported by current evidence:

- One application localization service reads data from top-level `locales/`.
- English is the canonical fallback for the currently cataloged messages.
- Ten release locale catalogs and two debug-only test locales are present and
  technically validated for those keys.
- Language selection is persisted in workspace preferences and does not enter
  render configuration or project semantics.
- The existing stable workflow/command identities remain unchanged.
- Main Home cards, top-level workflow navigation, selected Quick Render and
  Chart Studio labels, and selected Presentation Studio shell text can switch
  live to catalog strings.
- Release packaging now stages locale resources and documents them; CI smoke
  checks are configured but not yet run against an artifact here.

Not supported by current evidence:

- “The whole application is translated.” Most screens and nearly all
  secondary content remain English.
- “CJK/Hangul/Cyrillic rendering is visually correct.” No screenshots or
  real UI inspection were available.
- “RTL layout works.” Only stress text generation exists.
- “Translations are human-quality.” They are unreviewed machine drafts.
- “Locale never changes rendered output.” Locale is excluded from renderer
  configuration by design and a serialization test passes, but the requested
  ten-locale real-render comparison was not run.
- “Packaged localization passed.” The release workflow and package smoke
  coverage were changed, but no installer/portable artifact was built or
  executed in this task.

The largest remaining implementation gap is extracting and translating the
rest of the application strings: all major workflow controls, dialogs,
inspector content, progress/status/error messages, settings pages, help,
accessibility labels, and command descriptions. The largest validation gaps
are real UI inspection in the five reference locales and package-artifact
execution. Until those are closed, the correct overall result is **PARTIAL**,
not complete localization support.
