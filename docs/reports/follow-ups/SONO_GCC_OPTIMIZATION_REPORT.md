# Sono-GCC optimization report — Follow-up #44

> Historical snapshot: this report records the v4→v5 optimization pass.
> Follow-up #45 verification advanced the compiler identity to v6, routed
> `Lerp`, `LerpClamped`, and all easing variants through WatchVm after an
> exact-bit NaN-payload mismatch, and reran real-engine/whole-chart checks.
> See [SONO_GCC_OPTIMIZATION_VERIFICATION.md](SONO_GCC_OPTIMIZATION_VERIFICATION.md)
> for current operation coverage and evidence. No general runtime-speedup
> claim was established.

**Result:** the pass found and reduced static and dynamic singleton-region
fragmentation and shortened cold compilation. It did **not** establish a
general Watch-runtime speedup. GCC remains optional, and WatchVm remains the
semantic oracle and fallback.

## Baseline and method

The baseline was the pre-change v4 release binary; the candidate is v5. Both
were built from this Windows x64 checkout with Rust 1.99.0 / MSVC target. The
host exposed 12 logical processors; querying the processor model was denied,
so no CPU model or utilization claim is made. Watch timings are release-mode,
one Watch frame at `time=1`, on real Larp 64x, Baumkuchen, and ARMAGEDDON
LevelData using the same Next RUSH Official engine and validation resource
pack. Each mode has five fresh-process runs. The benchmark disables per-op
`BTreeMap` accounting (`-NoVmAccounting`), retains Watch execution, and
reports the median and observed range. These are isolated Watch timings, not
end-to-end render latency. The exact CSVs and logs are in the local, currently
untracked
`artifacts/sono-gcc-optimization/` directory.

The fixtures do not include Dreamer in this pass: that LevelData was not
available in the current local fixture set. The checked engine/LevelData and
resource pack were held constant between v4 and v5. Both v4 and v5 Draw hashes
match for all three fixtures.

## Profile findings: Larp 64x

The graph contains 213,221 nodes. The original static region metadata had
25,834 regions and 43,404 unique eligible native nodes. Static native-node
region sizes were:

| Native nodes per region | v4 regions | v5 regions |
|---|---:|---:|
| 1 | 14,979 | 0 |
| 2 | 3,830 | 3,830 |
| 3–4 | 5,451 | 5,451 |
| 5–8 | 1,244 | 1,244 |
| 9–16 | 276 | 276 |
| 17–32 | 54 | 54 |
| 33+ | 0 | 0 |

Before filtering, mean region size was 2.082, median 1, p90 4, p95 5, p99 15,
and maximum 22 native nodes. After filtering, there are 10,855 regions, with
mean 3.574, median 3, p90 6, p95 8, p99 16, and the same maximum 22. The
graph still has 43,404 statically eligible nodes; 28,425 are emitted into v5
regions. Singleton nodes are evaluated by WatchVm.

For the profiled Larp frame:

| Dynamic measure | v4 | v5 | Change |
|---|---:|---:|---:|
| Native region entries | 134,914 | 34,360 | −74.5% |
| VM-to-native scalar cuts | 236,089 | 106,039 | −55.1% |
| Region entries per 1,223 callbacks | 110.3 | 28.1 | −74.5% |
| Scalar cuts per 1,223 callbacks | 193.0 | 86.7 | −55.1% |
| Watch node evaluations | 2,219,199 | 2,219,199 | unchanged |

Dynamic executions by region size were 100,554 singleton / 130,050 cuts,
17,130 size-2 / 32,264 cuts, 13,897 size-3–4 / 49,630 cuts, 3,021 size-5–8 /
19,718 cuts, 120 size-9–16 / 587 cuts, and 192 size-17–32 / 3,840 cuts.
In v5, the singleton bucket is zero; the remaining buckets are unchanged.
No region used heap cut-input storage beyond the same 812 overflow-input
regions in this frame.

The highest-frequency VM cuts in the baseline were `Get` (199,475), `Set`
(25,379), `GetShifted` (8,454), `SetMultiply` (1,130), `IncrementPost` (684),
and `If` (651). After filtering, the leading counts were `Get` (93,541), `Set`
(6,969), `GetShifted` (4,887), `If` (420), and `SetMultiply` (174). The
highest-crossing baseline callback was `Preprocess/NormalTapNote` (45,051
region entries, 82,886 cuts); after filtering it was 10,728 entries and
38,794 cuts. Detailed histograms are printed by `run-watch --profile` and
captured in the saved logs.

These counts establish that singleton work was a major source of call
fragmentation. They do **not** establish that all removed crossings were the
dominant cost in wall-clock time.

## Changes

- Sono-GCC compilation now emits only regions with at least two direct native
  operation nodes. A singleton remains ordinary WatchVm work; native-region
  dispatch retains a defensive size check. The v4 cache key is invalidated,
  so old persisted DLL metadata is not loaded under v5.
- Compiler backend identity changed from `sono-gcc-codegen-v4` to
  `sono-gcc-codegen-v5`. This invalidates old persistent native artifacts; the
  runtime ABI did not change.
- Diagnostic profiling records dynamic region-size executions, cuts,
  singleton bypasses, hot VM cut operation kinds, and callback/archetype
  hotspots. These maps are collected only with Watch profiling enabled.
- The benchmark script accepts `-NoVmAccounting` so repeated timing runs do
  not include the operation-count map used for semantic diagnostics.
- Tests protect bucket boundaries, singleton routing, dynamic-address memory
  semantics, overflow inputs, lazy/effect ordering, exact native operation
  behavior, cache coordination, and deferred GUI precompile behavior.

No host-runtime ABI, callback-scale compilation, control-flow compilation,
memory bypass, or graph IR was introduced. `Get` and `Set` dominate the
remaining VM cuts, but bypassing WatchVm memory semantics would require a
versioned, panic-contained host ABI and adversarial checks for bounds,
aliasing, memory spaces, writes, and ordering. The measurements did not
separate the cost of each individual cut or FFI transition enough to justify
that higher-risk change in this pass. Lazy branches, effects, memory, and
runtime services therefore remain authoritative WatchVm operations.

No unsafe code was added. Floating-point formulas and strict compiler options
are unchanged; exact constant bit encoding and the existing Elastic NaN
payload safeguard remain. No cache, time, SFX, particle, Draw, or Watch
semantics changed.

## Compilation and cache footprint

For a cold Larp graph compile, v4 took 66.416 seconds in rustc compilation;
v5 took 27.524 seconds, a 58.6% reduction. The v5 first-process run also
reported 104 ms cache lookup and 106 ms DLL load, while later warm-process DLL
loads were 5–6 ms. These are individual cold/warm observations, not a
distribution.

| Persisted artifact | v4 | v5 | Change |
|---|---:|---:|---:|
| Native regions | 25,834 | 10,855 | −58.0% |
| `regions.json` | 5,734,602 B | 2,674,255 B | −53.4% |
| Native DLL | 856,576 B | 462,848 B | −46.0% |
| Full `graph.json` | 11,493,901 B | 11,493,901 B | unchanged |

The full graph remains part of the existing cache identity and was not
rewritten or omitted. ABI version is unchanged; backend version in the key
prevents loading v4 code under v5 selection semantics.

## Warm Watch timing results

All values are milliseconds, median [minimum–maximum], five fresh processes
per mode. Candidate change is v5 GCC p50 versus v4 GCC p50.

| Fixture | v4 VM | v4 GCC | v5 VM | v5 GCC | v5 GCC vs v4 GCC |
|---|---:|---:|---:|---:|---:|
| Larp 64x | 104 [102–122] | 97 [95–98] | 104 [101–110] | 97 [95–103] | 0.0% |
| Baumkuchen | 467 [462–480] | 404 [394–411] | 462 [460–469] | 416 [413–430] | +3.0% |
| ARMAGEDDON | 1,327 [1,319–1,372] | 1,147 [1,139–1,161] | 1,311 [1,307–1,331] | 1,200 [1,167–1,214] | +4.6% |

GCC remains faster than VM in the v5 medians on these three one-frame
workloads (6.7%, 10.0%, and 8.5%, respectively), but its v4-to-v5 warm Watch
timing did not improve on Baumkuchen or ARMAGEDDON. No general runtime
speedup is claimed. The larger-chart timing increases are visible in the
five-run ranges and remain a measured limitation to investigate; the region
policy is retained here for the substantial cold compile and artifact-size
reductions, not on a claim of faster steady-state execution.

Whole-render elapsed-time profiling was not separately captured for the
current v4/v5 comparison. One-frame Watch timings must not be presented as
whole-render timing. CPU utilization, per-call FFI cost, native allocations,
argument-copy bytes, source-generation time, and per-operation time were not
measured. `--profile` timing is diagnostic only: the instrumented Larp frame
was 223.532 ms in the earlier profiler and 235.468 ms with detailed telemetry;
the current detailed profile was 163.614 ms. This overhead is too large and
variable for those runs to serve as normal performance timings. The
unprofiled five-run measurements above are the timing evidence.

## Render and semantic validation

On the same Larp 64x input, v4 interpreter, v4 Sono-GCC, v5 interpreter, and
v5 Sono-GCC produced 20-second, 20-frame, 16×16 H.264/AAC MP4 files. FFprobe
reported matching 20.000-second video and audio streams at 1 fps. All four
files were 150,860 bytes and had SHA-256
`E4573C88DBB4801BC341C50E37319F8D87BD28FCE218C91DBBCD8A449C504FC7`. The
SFX sequence hash was `065623858bbffa464f9c2da960afbeefd8d4586b` in all modes,
with 93 scheduled effects, 3 loop starts, and 3 loop stops. The interpreter
Draw hash at Watch time 1 was
`f06217b8eafd30644a7f2a36ae0c94e4e4ad4fdf` for both compiler versions.

All four modes verify deterministic low-resolution segment output and SFX
scheduling. In addition, v5 interpreter and v5 Sono-GCC each ran the
`--whole-chart` workflow on this Larp 64x fixture. Both full-chart exports
were 20 seconds and 459,559 bytes, with matching H.264/AAC streams and SHA-256
`A5556DECF629E5434FF38EE96DAE50BC91173E9B7FD48A4C04F941FCE229B881`; the
SFX sequence hash and event counts also matched. On the final v5 release
binary, single external process wall-clock observations were 2.967 s for VM
and 3.114 s for GCC. This
low-resolution single pair is evidence that the whole-chart paths match, not
a statistically supported render-speed claim. No gameplay screenshot was
visually inspected for this optimization pass.

## Validation status

- `cargo fmt --all -- --check`: passed.
- `cargo check --offline`: passed.
- `cargo test --offline --lib sono_gcc::tests`: 24 passed after adapting
  synthetic fixtures to the minimum-region policy.
- Full `cargo test --offline`: passed on the rerun: 418 library, 27 CLI, 3
  `arctan2`, 101 renderer, and 9 scheduler tests; 8 ignored; doc tests empty.
  The initial run identified a GUI deferred-precompile fixture that contained
  only one native operation; it was updated to exercise a genuine two-node
  region. The final run had no failures or abnormal process aborts.
- Focused post-cleanup checks: all 24 Sono-GCC tests and all 9 WatchRuntime
  UI-configuration tests passed; `cargo check --offline` passed.
- `cargo clippy --offline --all-targets`: exit 0. It reports existing
  repository warnings (196 in the full lib-test compilation); the new
  callback-metrics helper no longer adds an argument-count warning.
- Final v5 `cargo build --release --offline`: passed after the telemetry
  aggregation cleanup (1m51s; the build notes that `rc.exe` is unavailable,
  so Windows executable version metadata is not embedded).
- Final release `run-watch --profile` smoke test on Larp 64x: passed with a
  persistent v5 cache hit, 10,855 regions, 34,360 dynamic entries, 106,039
  VM-to-native cuts, 1,223 callbacks, 111 Draw commands, and the baseline raw
  Draw hash. The hot-operation and callback histograms matched the saved v5
  profile.
- Whole-chart v5 VM/GCC MP4 parity and v4/v5 × VM/GCC 20-second segment
  parity: passed as described above. The whole-chart fixture is 20 seconds;
  a separate longer-chart SFX-tail case was not rerun.
- GUI was not interactively exercised. It continues to use the existing
  background compile and VM-on-miss path; no GUI startup behavior was changed.
- The v4 comparison executable/cache remain in local artifacts for
  this A/B pass; they are not a second shipped execution mode. WatchVm remains
  the supported interpreter, correctness oracle, and safe fallback.
- FFmpeg was sourced from a local 8.1 archive and installed only under the
  validation artifact directory. No network provisioning was used; the two
  local executables occupy about 446 MB and remain untracked.
- Normal VM fallback is retained; GCC remains optional and Windows-only.

## Reproduction

The validation inputs and old/new release binaries are retained in the local,
currently untracked `artifacts/sono-gcc-optimization/` directory. The benchmark SCP
was assembled only for this validation from the local packs containing the
`NR-V3-S1-EN` skin/effects and `NexintWaterMark` particle.

Example warm Watch benchmark command (run from the repository root; adjust
paths to the local fixtures):

```powershell
.\scripts\benchmark_sono_gcc.ps1 `
  -Name larp64x `
  -Engine 'C:\path\to\Next RUSH Official.zip' `
  -Level 'C:\path\to\larp 64x.json.gz' `
  -Resources 'artifacts\sono-gcc-optimization\compile-filter\benchmark-resources.scp' `
  -NoVmAccounting `
  -Renderer 'artifacts\sono-gcc-optimization\compile-filter\renderer.exe' `
  -OutputDirectory 'artifacts\sono-gcc-optimization\rerun'
```

The local FFmpeg 8.1 `ffmpeg.exe`/`ffprobe.exe` pair is under each binary's
portable `UserData\dependencies\ffmpeg\8.1` directory. To rerun the segment
export, use the same engine, resource, level, and music inputs, both
`--vm interpreter` and `--vm sono-gcc`, with
`--duration 20 --fps 1 --width 16 --height 16 --backend cpu --no-ui
--no-particles --no-bgm --sfx --no-notifications`, then compare FFprobe
stream metadata, SHA-256, and the SFX sequence hash. These are intentionally
small deterministic regression outputs, not production-resolution videos.
For the complete inferred chart duration, replace `--duration 20` with
`--whole-chart` and repeat for both VM modes.

## Follow-up opportunities

The next optimization should instrument costs within memory cuts and region
calls without turning diagnostic timing into normal-render overhead. If the
measurements show a material opportunity, a narrow versioned host ABI can be
evaluated with memory-space, bounds, aliasing, event-order, cancellation, and
panic-boundary differential tests. Callback-scale/control-flow compilation,
IR construction, and broader native memory semantics remain unimplemented;
they were not inferred safe from this region-size experiment.
