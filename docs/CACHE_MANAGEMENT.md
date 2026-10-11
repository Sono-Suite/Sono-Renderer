# Cache and generated artifact management

The cache manager is shared by the Sono-Renderer CLI and desktop GUI. It only manages artifacts that the renderer can regenerate. It does not scan or delete profiles, settings, queued-job history, source assets, rendered videos, Cargo build output, or retained `artifacts/` diagnostics and benchmark evidence.

## Inventory

| Data | Producer / consumer | Identity and validity | Location / lifetime | Managed here? |
|---|---|---|---|---|
| Sono-GCC native library and region metadata | `sono_gcc` compiler; Watch renderer | SHA-1 key over serialized Watch graph and renderer/compiler/runtime ABI, rustc, target, and compilation flags. Manifest hashes and lengths cover the graph, native library, and metadata. | `%LOCALAPPDATA%\Sono-Renderer\cache\sono-gcc-v2\<key>`; temporary-directory fallback when the primary store cannot be used. Persistent across runs. | Yes |
| Loaded Sono-GCC programs | Watch renderer | Same graph key; process-local `Arc` cache, at most four entries. Persistent DLL loads retain an OS read lease. | Process memory and the compiler's temporary DLL directory. | Yes; idle references only |
| Decoded SFX PCM | Audio mixer and duration inference | Content digest of the exact encoded resource bytes; per-content single-flight decode gate. | Process-local LRU, capped at 128 clips and 256 MiB retained PCM. Different resources decode independently; active `Arc` users remain valid after eviction. A cache generation prevents an in-flight decode from repopulating after clear. | Yes |
| WGPU source-atlas textures | GPU renderer | Content digest includes RGBA bytes and dimensions. | Device/process-local LRU, capped at eight atlases and 512 MiB estimated texture bytes. | Yes; dropping the cache reference does not invalidate a texture already held by a frame |
| WGPU pipelines/shaders | GPU renderer | Created from static renderer shader and device configuration. | Device lifetime; no persistent shader/pipeline cache is configured (`wgpu` cache is `None`). | No disk entry exists |
| Prepared renderer/session data | Export/render path | Scoped to the render/session; no persistent prepared-engine or decoded-resource disk cache was found. | Session memory; released with its owner. | No persistent category exists |
| FFmpeg intermediates | Export media helpers | Per-operation temporary files. | OS temporary directory, owned by `NamedTempFile` and removed when its owner drops. | Not persistent cache |
| Cargo `target/` | Cargo/rustc | Cargo fingerprint/build rules. | Repository build directory. | No; managed with Cargo, not the application |
| Profiles, config, render-job store | GUI/CLI shared configuration and job manager | Versioned user-facing documents; not safely regenerable cache. | Application data/configuration locations. | Never |
| `artifacts/`, UI/watermark PNGs, benchmark CSVs, render outputs | Tests, benchmark/validation examples, and users | Intentional evidence or user output; often expensive and not reconstructible without the exact private fixtures. | Repository `artifacts/` or user-selected output paths. | Never |
| Managed FFmpeg binaries | FFmpeg provisioning | Pinned distribution and executable validation. | Managed dependency/add-on location. | Not cache; do not remove with cache controls |

In the final read-only inventory for this pass, the machine had one app-data Sono-GCC entry (18,085,853 bytes) and 93 temporary-store entries (60,814,461 bytes), for 94 entries and 78,900,314 bytes total. Deep validation found six schema-4 entries valid, 65 schema-3 and 22 schema-2 entries incompatible, and one zero-byte partial entry with no manifest. The six valid temporary entries (410,240 bytes combined) came from the first two test-suite runs before unit tests were redirected to an automatically cleaned temporary cache; the final test run did not add entries to the user store. They were left in place because the external temp cache was read-only in this environment. Old/incomplete entries are ignored by execution and are safe to remove only through an explicit manager action. These are observations from this machine, not expected defaults. The repository's retained diagnostic/output tree is much larger and intentionally outside this cache manager.

## Persistent Sono-GCC identity and format

The current manifest format is `sono-renderer-cache-entry`, schema **4**. It records the cache key, creation time, serialized graph digest/length, renderer package version, compiler backend version, runtime ABI, rustc identity, host target, compiler flags, library digest/length, region-metadata digest/length, and region count. The region-metadata payload has its own schema. The target triple includes OS and architecture, so native artifacts from another target cannot be reused.

Manifest reads are capped at 1 MiB and each graph/DLL/metadata payload is capped at 256 MiB. Oversized generated entries are not published; oversized or mismatched on-disk entries become misses before deserialization/use.

Watch graph content is the compiler input. Skin, level, audio, output, and rendering settings are intentionally absent from the key because they do not affect generated Watch scalar code. A renderer/compiler/ABI/target/flag change makes the entry incompatible. Schema 3 and unknown schemas are reported as incompatible and rebuilt on use; bytes are never deserialized as the current manifest schema.

List/refresh performs a bounded metadata/path/size scan and reports current entries as **unverified**. Explicit `cache validate` verifies payload hashes, graph identity, metadata shape and indices, target/compiler compatibility, and safe file names. Both are read-only and do not create lock files. They use existing shared locks when available; if no lock state exists they inspect a best-effort snapshot, while runtime loading independently revalidates under its use lease. Validation is not performed at application startup or on the render hot path.

## Writes, concurrency, and recovery

Each entry has a stable cross-process lock under the cache base's `.locks` directory. A brief global manager lock coordinates creation and removal of per-entry lock files; compilation and use locks are per graph, so unrelated graph entries remain concurrent. Duplicate requests wait for the same per-entry lock, then recheck the process and disk caches before compiling. A waiting consumer does not cancel the owner’s compilation.

Cache payload files are written to unique temporary files in the entry directory, flushed, and renamed into place. The manifest is removed before replacing any payload and published last as the commit marker. A crash before manifest publication therefore produces a miss, never a valid partial hit. Kernel file locks are released on process exit, so a crashed compile does not leave a permanent “busy” sentinel.

A disk-loaded program holds a shared read lease for its lifetime. Removal and pruning take the per-entry exclusive lock and refuse active entries; the GUI and CLI report skipped active entries. Cache clearing never invalidates an `Arc` already used by a renderer frame or audio mix. Corrupt or incompatible native entries are ignored by execution, which compiles again or falls back to Sono VM; cache persistence failure does not make the Watch VM unavailable.

Creation time is stored in the manifest. Last use is tracked by the `last-used` file timestamp when a persistent program is loaded or reused from the process cache. Entries whose last-use time is unknown use creation time for explicit retention ordering.

## CLI

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

`appdata:` and `temp:` identify the managed store in an entry ID. `list`, `stats`, and `validate` support JSON for scripts. `validate` without an ID checks every persistent entry. `rebuild` requires an intact, identity-valid stored graph; if that graph is damaged, rerun `compile-engine <engine.zip>` or select the engine again so the source Watch graph can be loaded.

The existing `renderer clear-gcc-cache` command remains available and keeps its previous persistent-cache-only meaning. `renderer cache clear` also drops safe process-local cache references unless `--disk-only` is supplied.

## GUI and retention

The existing **Render / video → Compiler cache** control remains. The **Cache manager** tab provides refresh, entry size/version/target/status/timestamps, full validation, per-entry removal, inactive-cache clearing, and an explicitly confirmed size prune. Its target starts at 10 GiB in each GUI session. No automatic age or size pruning is enabled; disk deletion occurs only after an explicit user action. Active entries are preserved even when the requested target cannot be reached.

The manager currently identifies Sono-GCC entries by Watch-graph digest; no engine or project display name is stored in the compiler manifest. GPU/PCM caches are listed as process memory categories with measured/estimated bytes and limits.

## Performance and limits

Cache list/size accounting runs only when requested by the user or a CLI command. It reads each managed entry's manifest and file metadata but does not hash multi-megabyte libraries. Full hashing is the explicit validate operation. The initial per-process rustc identity is memoized, while the exact serialized Watch graph remains part of identity.

The standard-library OS file-lock path is currently exercised on Windows, which is the only installed build target in this environment. Unit tests route Sono-GCC disk artifacts through an automatically cleaned temporary cache root, so the test suite does not grow or clear a user's persistent cache. Cross-platform persistent Sono-GCC caching remains unavailable because native compilation is Windows-only. The compiler cache uses SHA-1 for compatibility with its established graph and payload identity; it is intended to detect stale/corrupt generated files, not to authenticate cache files against a malicious same-user process. No secret is stored in cache metadata.
