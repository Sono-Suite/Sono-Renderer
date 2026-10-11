# Sono-Renderer Post-Integration Follow-up #1

## Cache and generated-artifact audit

The repository has one persistent application cache category today: Sono-GCC's compiled Watch graph DLLs and their graph/region metadata. No persistent prepared-renderer, decoded-resource, shader, or pipeline cache was found. Other reusable state is process-local or temporary and has a bounded owner/lifetime.

| Data | Producer and consumer | Identity / lifetime | Location and treatment |
|---|---|---|---|
| Sono-GCC native DLL, Watch graph, region metadata | Sono-GCC compiler; Watch renderer | SHA-1 graph key includes serialized Watch graph, renderer package version, compiler backend, runtime ABI, rustc identity, target triple, and compiler flags. Manifest hashes and lengths validate payloads. | `%LOCALAPPDATA%\Sono-Renderer\cache\sono-gcc-v2\<key>` with `%TEMP%` fallback. Persistent, managed cache. |
| Loaded Sono-GCC programs | Watch renderer | Exact graph key; process-local `Arc` cache bounded to four programs. Disk-loaded programs retain a shared OS read lease until released. | Process memory and temporary DLL load location. Safe idle-reference eviction only. |
| Decoded SFX PCM | Audio mixer and duration inference | SHA-1 of exact encoded resource bytes; per-content single-flight decode gate. | Process-local LRU: 128 clips / 256 MiB retained PCM. Different clips decode independently; active clips survive eviction through `Arc`. A generation counter prevents in-flight decodes from repopulating the cache after clear. |
| WGPU source-atlas textures | GPU renderer | Content identity includes RGBA bytes and dimensions. | Device-local LRU: eight atlases / 512 MiB estimated texture bytes. In-flight frame references survive eviction. |
| WGPU pipelines/shaders | GPU renderer | Device and static renderer shader configuration. | No persistent pipeline cache is configured (`wgpu` cache is `None`). |
| Prepared renderer/session data | Render/export path | Scoped to an active render/session. | Memory lifetime follows its owner; no persistent cache found. |
| FFmpeg intermediate files | Export/media helpers | Per-operation temporary files. | OS temp directory, owned by `NamedTempFile`/`tempfile` and removed on drop. |
| Cargo `target/` | Cargo/rustc | Cargo build fingerprints. | Build state, not app cache; untouched by manager. |
| Profiles, settings, render jobs/history, source assets, render output, retained diagnostics | GUI/CLI and users | Durable records or user-selected output. | Explicitly outside manager scope and never deleted by its operations. This includes the large retained `artifacts/` evidence trees. |
| Managed FFmpeg binaries | Dependency provisioning | Pinned and executable-validated distribution. | Managed dependency, not disposable cache. |

The final read-only release-CLI inventory observed **94 entries / 78,900,314 bytes**: one app-data entry (18,085,853 bytes) and 93 temp-store entries (60,814,461 bytes). Deep validation found six schema-4 entries valid, 65 schema-3 and 22 schema-2 entries incompatible, and one zero-byte partial entry missing its manifest. The six schema-4 entries totaling 410,240 bytes were produced by the first two complete test runs before the unit-test cache was redirected to an isolated `TempDir`; the final complete run did not add to the user's store. Their exact IDs are `temp:2ccf3d35cbf1be66fe47fa43bfcfe4194405b206`, `temp:7f01cf3d6a756ab0126b969d8dfc69d437ba6d5a`, `temp:a8fb3edfb0621ab71127e37a5f9ef23c7838bcf0`, `temp:ad6bb2c7fa31180420fadc371d91c0e4b97cef5d`, `temp:af16b8b9330ed2e6ba1760c1e0c3004f849649be`, and `temp:f2a5661e71d1e9d4e86d45d4c565db60982ddd36`. They are regenerable cache entries and can be removed individually with `renderer cache remove <ID>`. They were left in place because this environment has read-only access to the external temp cache store. Validation otherwise left files untouched. Older/incomplete entries are cache misses and are ignored by execution.

## Identity, format, and invalidation

Sono-GCC's current persistent format is `sono-renderer-cache-entry`, schema **4**. The manifest records its format/schema, graph digest and length, creation time, renderer version, compiler backend version, runtime ABI, rustc identity, target triple, build flags, DLL digest/length, region-metadata digest/length, and region count. Region metadata has its own schema. The target triple distinguishes operating system and architecture.

Only Watch graph/compiler inputs are in the key. Skin, level, audio, output, and rendering settings do not affect the generated Watch scalar program and are excluded. A graph, renderer, compiler, ABI, rustc, target, flag, or format mismatch is incompatible and causes a miss/recompile or interpreter fallback. Schema 2/3 and unknown schemas are not deserialized as schema 4. SHA-1 is retained for established content identity and accidental corruption detection; it is not an authenticity/signature mechanism against a malicious same-user process.

Manifest reads are capped at 1 MiB and each serialized graph, DLL, and region-metadata payload at 256 MiB. Oversized generated entries are rejected before payload publication; oversized disk declarations are rejected before payload access.

Cache entry IDs are store-qualified (`appdata:<40-hex-key>` or `temp:<40-hex-key>`), so manager operations resolve a validated direct child of an approved cache root. The compiler manifest does not store an engine/project display name, so inventory identifies entries by Watch graph digest prefix.

## Writes, recovery, and concurrency

- Payloads are written to unique same-directory temporary files using `create_new`, flushed with `sync_all`, and atomically renamed into place.
- The old manifest is invalidated before payload replacement. The new manifest is published last as the commit marker. A crash before that point is a miss, not a hit on a partially written entry.
- Stable lock files are outside removable entry directories. A manager lock coordinates creation/removal of per-key lock files; independent graph keys retain independent locks.
- Cross-process compilation takes an exclusive per-key lock, then rechecks process/disk cache after waiting. Readers take a shared use lease. Removal/pruning takes the exclusive lock and skips active entries. OS locks are released after process termination, so there is no stale `compile.lock` sentinel.
- A per-graph in-process single-flight gate now also covers cold compile work when both persistent stores are unavailable. The weak gate registry retains only identities with active requests. Same-key requests share the completed process cache result; unrelated graph keys can progress independently. One consumer cannot cancel the shared synchronous compile.
- Decoded SFX uses the same per-content pattern: the LRU mutex is not held while FFmpeg decodes, duplicate requests for one byte identity share the in-flight decode, unrelated clips can decode concurrently, and cache clearing advances a generation so a late result is not reinserted.
- Corrupt, incompatible, unsafe-path, partial, oversized, or unrecognized payloads are cache misses. Cache persistence/validation failure does not disable WatchVm, which remains the semantic fallback.
- Inventory and explicit validation are read-only and do not create lock files. If stable lock metadata exists, they take a shared snapshot lock. If not, they inspect a best-effort snapshot; the renderer independently validates under its runtime lease before reuse.

## Retention and user controls

No automatic age or size pruning runs on startup or during rendering. The GUI's manual size target starts at 10 GiB per session. Explicit pruning orders by last-used time and falls back to creation time; active entries are skipped even when that means the requested target cannot be reached. Clearing memory caches only drops manager references; active `Arc` audio/GPU/program references remain alive.

The main GUI now has a **Cache manager** tab using the same `cache_manager` implementation as the CLI. It lists category, graph identity, size, creation/last-use time, schema, target, and validity status. It provides refresh, validate all, remove selected, clear inactive cache, and confirmed manual size pruning. The existing **Render / video → Compiler cache** control remains.

The CLI adds:

```text
renderer cache list [--json]
renderer cache stats [--json]
renderer cache inspect STORE:SHA1
renderer cache validate [STORE:SHA1] [--json]
renderer cache remove STORE:SHA1
renderer cache clear [--disk-only]
renderer cache prune --max-bytes BYTES
renderer cache rebuild STORE:SHA1
```

`renderer clear-gcc-cache` remains as the previous persistent-cache-only alias. GUI and CLI share inventory, validation, deletion, retention, and rebuild logic. Memory caches are process-local; a CLI process cannot clear another already-running GUI process's RAM caches. Disk operations cover only the Sono-GCC cache category currently present.

## Measurements

**Measured in this pass:** after the test run, three release `cache list --json` samples over 94 entries had a median of **159.0 ms**. Three read-only `cache validate --json` samples had a median of **180.8 ms** and reported 6 valid, 87 incompatible, and 1 corrupt entry. The nonzero validation exit is expected because the existing store contains old/incomplete entries. No full scan occurs at startup or per frame.

**Existing real-fixture measurements, not rerun as a cold compile in this pass:** the current GCC performance report recorded Larp cold compilation at 64.562 s (1.205 s region codegen, 63.353 s rustc), and Next Sekai at 42.765 s (0.903 s codegen, 41.859 s rustc). Three fresh-process Larp cache hits took 127–142 ms for lookup, including 93–103 ms graph identity, 9–11 ms disk validation, 24–28 ms metadata reconstruction, plus 13–18 ms DLL load. The final integration report's isolated one-second Larp Watch run measured 179 ms cold and 138/143 ms warm Watch execution; those figures describe Watch execution and process time, not just cache lookup. See [GCC performance report](SONO_GCC_PERFORMANCE_REPORT.md), [GCC implementation report](SONO_GCC_IMPLEMENTATION_REPORT.md), and [final integration report](SONO_RENDERER_FINAL_INTEGRATION_REPORT.md) for setup and limits.

## Validation

Final checks on this code state:

- `cargo fmt --all --check` — passed.
- `cargo check --offline --all-targets` — passed.
- `cargo test --offline --all-targets` — passed on final behavior: **185 library tests, 17 CLI tests, 3 `arctan2` tests, 101 fixture tests, and 9 scheduler tests**; three private-data diagnostic tests were intentionally ignored.
- Focused cache tests cover path-safe IDs, read-only inspection without lock-file creation, active-entry deletion refusal, user-data preservation, same-key locking and unrelated-key progress, oldest-first retention with timestamp fallback, bounded SFX LRU behavior, per-content SFX gates and clear generations, atlas identity, atomic payload publication, schema and payload-size rejection, exact graph-key changes, isolated test cache roots, and the per-key cold-compilation gate. The real-engine tests also include concurrent precompile deduplication and persistent warm-hit behavior.
- `cargo clippy --offline --all-targets` — passed with existing repository warnings (deprecated/dead instrumentation, style, and test-construction lints); it did not report a cache-manager-specific warning.
- `cargo build --release --offline` — passed.
- Release CLI `cache list --json` and `cache validate --json` — exercised read-only against the current stores; no remove, clear, prune, or rebuild command was run against the user's existing cache roots.

The GUI was compiled but not interactively click-tested. One earlier all-target attempt terminated with Windows `STATUS_HEAP_CORRUPTION` and no Rust assertion report; the focused audio suite passed and a complete immediate rerun passed, so the failure did not reproduce and remains an intermittent native-test/environment flake. Three private-corpus diagnostics remained ignored by their existing test annotations. Persistent native compilation is Windows-only; cross-platform behavior outside this environment is not verified here. Cross-process deduplication requires both processes to see a common writable cache root; when no shared store is writable, only same-process requests can share the in-flight result.

## Reproduction

From the repository root:

```powershell
cargo fmt --all --check
cargo check --offline --all-targets
cargo test --offline --all-targets
cargo clippy --offline --all-targets
cargo build --release --offline
target\release\renderer.exe cache list --json
target\release\renderer.exe cache validate --json
```

`validate` exits nonzero while incompatible/corrupt entries remain. The commands above do not delete cache contents; use `remove`, `clear`, `prune`, or `rebuild` only when that explicit cache operation is intended.
