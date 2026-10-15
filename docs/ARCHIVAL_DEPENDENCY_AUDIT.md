# APK inspection infrastructure archival audit

Date: 2026-10-15.

## Scope

These exact paths are optional archival research/development material:

| Repository-relative path | Classification |
| --- | --- |
| `TestingSuite/android-sdk-live/` | Android SDK, emulator, AVD and inspection tools |
| `TestingSuite/Sonolus Inspection/` | Private client inspection evidence and associated tools |
| `sono-apk/` | Repository-root private APK research location |

None is an application resource directory or packaging input. All three can be
absent simultaneously. No production change was justified by the dependency
scan; no test was deleted, weakened, or newly skipped. The original directories
were not moved, deleted, compressed, or modified by this audit.

The validation did uncover one unrelated compilation defect:
`examples/ui_validation.rs` had not initialized four recently added
`UiBitmapFont` fields. The initializer now supplies empty `glyph_scales` and
`glyph_advances`, and `None` for `center_spacing` and `glyph_max_dimension`,
matching the fields' existing serialization defaults. This restores normal
test/example compilation without changing renderer behavior or fixture data.

## References found

| Location | Finding and action |
| --- | --- |
| `src/`, `tests/` | No references to the three paths, APK/Dex/IL2CPP loading, Android SDK environment variables, or ADB execution. No current Rust test requires restoring these archives. |
| `Cargo.toml`, `Cargo.lock`, `build.rs`, `.cargo/` | No archival-path dependency. The native Windows resource compiler is **Windows SDK** `rc.exe`, not an Android SDK tool. Build-generated locale includes come from `locales/`. |
| `scripts/`, `.github/`, `packaging/` | No archival-path dependency. The Windows package copies the executable, `assets/`, UI/watermark profiles, locales and selected documentation; it does not package the inspection trees. |
| `assets/`, `ui-profiles/`, `watermark-profiles/`, web source/build | No matching paths or APK-loading requirement. Runtime loading uses selected engine/resource files and the application resource root, not an automatic APK search. |
| `docs/PARTICLE_COMPATIBILITY_HANDOFF.md` | A historical private evidence link points inside `TestingSuite/Sonolus Inspection/1.1.4/findings/`. Preserved and explicitly marked archival. Reading that evidence requires restoring the archive. |
| `docs/PRE_UI_STORAGE_AUDIT.md` | Historical storage table and an LLVM installer path under the inspection tree. Preserved; marked as an earlier storage snapshot rather than a current dependency list. |
| `.gitignore` | Root `/sono-apk/` remains excluded as private reference material. This is an exclusion, not a runtime reference. |
| `artifacts/base-clear/baseline-source/TestingSuite/` | Existing historical trees named `android-sdk-live/` and `Sonolus Inspection/` contain old absolute paths in emulator configuration, inspection metadata and tool logs. These are not active application inputs. They were not modified. They are outside the requested archival scope; inspect their physical allocation/hard-link status separately before estimating total space reclaimed. |
| Historical snapshot `.gitignore` files under `artifacts/` | Private APK exclusions in old source snapshots; no executable dependency. Preserved. |

Searches included ignored files outside the archival directories and generated
build/dependency trees. The initial broad search was refined to exclude archival
copies themselves: their own old emulator paths and installer logs are expected
historical references. Active code/configuration searches had no file-size cap.
Broad historical text searches used a 2 MiB per-file cap; this is not a forensic
scan of every binary or every generated file on the machine.

## Indirect dependencies and test requirements

Reviewed filesystem/resource loading, `CARGO_MANIFEST_DIR`, compile-time
`include_str!`/`include_bytes!`, child-tool commands, environment lookup,
manifest/profile paths, package inputs, and path construction. Searches also
covered standalone APK/Android/Dex/IL2CPP terms to catch paths assembled from
fragments rather than only literal full paths. No hidden APK-to-runtime asset
resolution or Android toolchain requirement was found.

The standard installed user-data directory was searched read-only for matching
JSON references: zero matches. No process environment PATH entry or inspected
Android/toolchain environment value referred to an archival directory.
Resource/configuration trees (`assets`, UI and watermark profiles, locales,
packaging and `web/dist`) contained no reparse points redirecting to retired
data. Thus no symlink/junction asset dependency was found either.

The independent fixtures below remain meaningful requirements:

| Test/workflow | Actual external/local requirement |
| --- | --- |
| `render_ui::tests::sono_overlay_ap_combo_assets_follow_the_lua_ap_branch` | Includes `TestingSuite/Sono-Overlay/pkg/sono-overlay/sekai.obj2` at **test compile time**. It does not read APK files. This existing compile-time fixture requirement was not changed. |
| Native HUD artwork/reference tests | `TestingSuite/Sono-Overlay/build/assets/combo/` and supplied application artwork. |
| Real note-scoring and chart/Watch tests | Independent Next Sekai/Next RUSH level, engine and resource fixtures. |
| Explicit real-MP4 tests | Independent chart/engine/resource fixtures plus FFmpeg/FFprobe. Ignored integration tests still require deliberate invocation; archival creates no new ignore. |
| Private Horizon/SDB particle evidence tests | Independent `TestingSuite/Horizon/` corpus and any separately documented reference sources. Existing ignored evidence tests remain available; they are not converted into mock tests. |
| Visual HUD comparisons | `TestingSuite/ui_ref/`, Sono-Overlay, or separately named reference recordings. These are outside this archival scope. |

This audit does not assert that removing **all** local fixtures will leave every
test runnable. It establishes the boundary for the three specified inspection
directories. No unrelated fixture was edited.

## Isolated validation method

Created `target/archive-independence/source/` from the 499 tracked files in the
current checkout. That view has no `sono-apk/`, no
`TestingSuite/android-sdk-live/`, and no `TestingSuite/Sonolus Inspection/`.
The absence of all three is checked before validation.

Read-only use of the independent fixtures is retained through directory
junctions to `Next RUSH`, `Next Sekai Engine`, `Sono-Overlay`, `ui_ref` and
`Horizon`. Managed dependencies and independent reference source trees are also
made available; none of these junctions targets an archival directory. The
historical `artifacts/base-clear` copies are not exposed in the isolated view.
Four independent fixture files are copied separately: `proseka_official+2.scp`,
`Next RUSH Official (2).zip`, `Test1.mp4`, and `Test2.mp4`. No APK or IPA is exposed.
Commands run with the isolated view as their working directory and compile-time
manifest root.

The process PATH is filtered to exclude the three archival names, and Android
SDK/AVD environment variables are unset for validation. Cargo's existing target
directory is reused to avoid duplicating the entire dependency build. Explicit
`CARGO_ENCODED_RUSTFLAGS` retains the normal single
`-C target-feature=+crt-static` setting: nesting a copied `.cargo/config.toml`
below the original checkout would otherwise concatenate the same rustflags.
This is a validation-environment adjustment, not a production build change.

Representative commands, from that view:

```powershell
$env:CARGO_TARGET_DIR = '<original checkout>/target'
$env:CARGO_ENCODED_RUSTFLAGS = '-C' + [char]31 + 'target-feature=+crt-static'
cargo test --locked --no-run
cargo test --locked --lib -- --test-threads=1
cargo test --locked --lib real_automatic_ap_chart_export_contains_gameplay_ui -- --ignored --nocapture
cargo build --locked --bin renderer
```

Logs and generated output remain under `target/archive-independence/` or the
isolated view's generated output directories. No archived data is copied into
production resources.

## Validation results

All three archival paths remained absent from the isolated view. Validation
recovered and resumed the interrupted run rather than repeating its successes.

| Check | Result |
| --- | --- |
| Isolated application build | Passed, `cargo build --locked --bin renderer`; `rc.exe` version-metadata warning only. |
| Test/example compilation | Passed after the four-field example initializer correction. |
| Library cases, combined recovered/resumed results | 505 passed, 6 failed, 10 existing ignored cases. The three particle fixture failures in the first partial log are superseded by their successful focused rerun. |
| CLI target | 28 passed, 2 failed. |
| `arctan2` | 3 passed. |
| `m0` core integration | 103 passed. |
| Watch scheduler | 10 passed. |
| Separate asset-import/export target | 2 existing ignored integration cases; no new ignore was introduced. |
| Doctests | Completed successfully; 0 cases. |
| Explicit real-chart MP4 export | Passed separately; H.264, 1920×1080, 60 FPS, 3 frames, 0.05 seconds, 224,309 bytes. Existing MP4 and FFprobe evidence were checked during continuation; this successful export was not rerun. |
| Particle resource regression | 3 passed using independent `proseka_official+2.scp`. |
| Preview/HUD regressions | Completed, including shared preview, CPU/WGPU comparisons, source-derived geometry/assets/spacing, timestamp animations, Glow/Light and profile save/reopen. |
| Formatting/diff checks | Passed: example rustfmt check and `git diff --check`. |

Across the normal targets: **649 passed, 8 failed, 12 existing ignored cases**
(669 cases accounted for). The explicit MP4 export was a separate invocation of
one normally ignored library case. This is **not** an all-green test suite.
`target/archive-independence/final-results.json` records per-target counts,
failure names and absence checks. No archival-path access failure was reported.

### Baseline profile failures

The isolated run exposes these existing failures:

- `config_profiles::tests::profile_crud_selection_reset_and_snapshot_round_trip`
- `config_profiles::tests::import_export_round_trip_is_versioned_and_never_exports_credentials`

Their configuration equality checks include the transient HUD
`profile_directory`: a freshly constructed configuration has `Some(resolved
ui-profiles path)`, whereas a serialized/reloaded configuration has `None` for
this skipped field. This is independent of APK/SDK files.

To distinguish this from an archival regression, the same compiled test harness
was temporarily copied beside the **original checkout's** application resources
and run there with `config_profiles::tests:: --nocapture --test-threads=1`.
All three original archival directories were present. Result: two passed, the
same two failed. The executable's location makes the authoritative application
resource resolver use the original resource root rather than the isolated one.
The temporary harness copy was removed afterward. No source/test assertion was
changed to conceal these failures. The original-root baseline log is
`target/archive-independence/profile-original-root-baseline.log`.

This audit therefore cannot claim a completely green repository-wide test
suite. The profile equality failures remain a separate test/persistence follow-up;
they are not dependencies that would be restored by retaining APK inspection data.

Investigation detail: `RendererUiConfig` derives `PartialEq` over its runtime
fields, but explicitly marks `profile_directory` and loaded asset/event state
`#[serde(skip, default)]`. `reference_default()` sets a profile directory;
configuration resource resolution intentionally clears it after resolving
embedded paths. The profile tests compare the entire in-memory configuration
instead of just persisted settings. The first profile failure's complete Debug
states match after excluding only `profile_directory`. This does not justify
adding the transient field to serialized user data, altering migrations, or
changing runtime path resolution. No such changes or assertion weakening were
made. The default profile's durable asset-pack path and logical `pack://`
identifiers remain present.

### Other existing failures recovered during continuation

The interrupted log also recorded:

- `render_history::tests::records_round_trip_and_keep_unknown_fields`: the same
  transient directory equality difference; complete Debug states match apart
  from that field.
- `render_manifest::tests::manifest_round_trip_hashes_inputs_and_redacts_application_settings`:
  the test starts with the active default HUD and adds a synthetic logo, leaving
  unloaded `pack://` references among its inputs. Its all-inputs-have-SHA256
  assertion fails.
- `render_manifest::tests::path_changes_do_not_change_content_identity_but_modified_and_missing_assets_report`:
  the synthetic test tries copying an unresolved logical asset identifier as a
  filesystem path and gets Windows error 123 (`InvalidFilename`).
- `job_cli_tests::cli_profile_job_keeps_resolved_snapshot_after_profile_changes_and_deletion`
  and `job_cli_tests::profile_cli_supports_crud_import_export_and_selection`:
  configuration equality assertions also fail in the CLI test target.
- `video_export::tests::single_watch_export_audio_pipeline_offsets_and_failure_cleanup`:
  the resumed run reaches `src/video_export.rs:2783` and fails
  `verification.can_reproduce_exact_backend` while checking a manifest created
  from a real VM history record. Earlier audio/export checks in this case ran
  before that assertion; later reproduction/retry/cancellation checks in the
  same case were not reached. Its original-root baseline is recorded separately
  in `export-manifest-original-root.log`; with all three archives present it
  fails at the same `verification.can_reproduce_exact_backend` assertion.

All three history/manifest failures and both CLI failures were individually
reproduced with the original application resources and all three archives
present, using temporary copies of their existing compiled test harnesses.
Those copies were removed afterward. Logs:
`history-manifest-isolated.log`, `history-manifest-original-root.log`, and
`cli-profile-original-root.log` under `target/archive-independence/`.
These are existing test/manifest issues, not newly required APK assets.

### Interrupted-run recovery

The original `tests.log` has no final suite summary. At continuation, no old test
process was running. Its 373 reported library cases were recovered: 358 passed,
8 failed, 7 ignored. The eight failures included the three particle tests whose
separate SCP fixture was initially missing from the isolated view; all three
had already passed their focused rerun after that fixture was copied.

The 148 unreported library cases were selected by exact test name and the
selection was checked against the harness's test list before running. Previously
completed expensive preview tests and the successful MP4 export were not
repeated. The previously unstarted CLI and integration targets were run
separately. `resume-selection.json`, `resumed-library.log`, and
`resumed-target-*.log` preserve this accounting. Source/build input comparison
confirmed the current and isolated copies of all 73 Rust/build input files
match. No serialization semantics were changed during continuation.

## Final archival verdict

**No dependency blocking archival of the three specified directories was found.**
The application builds, real preview/HUD/core tests run, and a genuine MP4 is
exported with all three absent from the source/resource view and their tools
excluded from PATH. Original directories and independent fixtures remain
untouched. Retired Android/APK inspection workflows and historical evidence
links naturally require restoring the archive.

**Full-product test certification is not green:** six library and two CLI tests
remain failing. All eight failures also reproduce with the original resources
and archival directories present. They concern transient configuration equality
or manifest input/reproduction handling; retaining APK/SDK inspection data does
not resolve them. They were not hidden, ignored, or repaired by changing
serialization behavior. No claim of clean-machine, packaged GUI, or every
possible external project/profile validation is made.

## Remaining limitations and archival effects

- Historical inspection links and manual Android/emulator/APK investigations
  require restoring the relevant archive. This is expected loss of access to
  retired research tools, not loss of application functionality.
- The pre-existing trees under `artifacts/base-clear/baseline-source/` remain
  outside this archival scope. Their physical allocation and hard-link sharing
  were not measured. No cleanup or archival of those trees was done.
- Arbitrary projects, portable installations, custom profiles or tools elsewhere
  on disk can contain user-authored paths not covered by this repository audit.
  If such a file explicitly selects an asset inside an archived tree, that asset
  must be restored or relinked. No such dependency was found in the audited
  active repository or standard installed JSON state.
- No Android device/emulator, packaged installer, or interactive GUI session is
  required or claimed by this archival audit. No new dependency on extracted
  APK assets was introduced.

## Files changed

| File | Change |
| --- | --- |
| `README.md` | Links to optional archival scope and this audit. |
| `REFERENCE_SOURCES.md` | Explicit archival policy for the three exact paths; independent fixture requirements retained. |
| `docs/PARTICLE_COMPATIBILITY_HANDOFF.md` | Marks the inspection evidence link as archival without rewriting historical findings. |
| `docs/PRE_UI_STORAGE_AUDIT.md` | Marks earlier storage/retention figures as historical. |
| `docs/ARCHIVAL_DEPENDENCY_AUDIT.md` | Dependency findings, recovered/resumed validation, baseline failures and limitations. |
| `examples/ui_validation.rs` | Adds four omitted bitmap-font initializer fields using existing defaults. |

No application Rust source, serialization schema, test assertion, engine fixture,
Sono-Overlay artwork, or archived directory was modified. Temporary validation
scripts/logs/views under ignored `target/archive-independence/` are generated
evidence rather than new application dependencies.
