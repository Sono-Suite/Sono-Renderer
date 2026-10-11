# Sono-GCC optimization verification — Follow-up #45

**Date:** 2026-10-08  
**Decision:** Sono-GCC remains an **experimental opt-in**. WatchVm remains the default, semantic oracle, and fallback.  
**Performance result:** **C. Workload-dependent**, with an important qualification: the available repeated v6 timing run was contaminated by concurrent host load, so it is not release-grade evidence of a general speedup.

## Executive result

The current v6 native path executes on real engine graphs. On the Larp 64x
profile it ran 34,431 native regions and made 105,410 ordered WatchVm-to-native
scalar cuts. The archived v4 profile recorded 134,914 entries and 236,089
cuts. That is 74.5% fewer entries and 55.3% fewer cuts relative to the v4
profile; v6 is close to the v5 profile (34,360 entries, 106,039 cuts).
Callbacks, evaluation counts, Draw count, and raw Draw hash match across the
profiled runs.

The adversarial exact-bit differential test found a real mismatch: native
`Lerp` could return a NaN with a different payload than WatchVm. v6 therefore
runs `Lerp`, `LerpClamped`, and all easing operations through WatchVm. The
compiler identity was bumped to v6 so native artifacts compiled under the old
operation routing cannot be reused. This deliberately gives up those native
operations to preserve exact VM semantics.

Interpreter and Sono-GCC full-chart Larp exports have identical SHA-256
(`A5556DECF629E5434FF38EE96DAE50BC91173E9B7FD48A4C04F941FCE229B881`),
matching H.264/AAC streams, and identical SFX sequence hash. In the single
whole-chart timing pair collected on the loaded host, GCC took 7.274 s and VM
7.035 s; the 3.4% difference is noise-sensitive and is not a supported speed
claim. Watch occupied at most 9.3% of the GCC export's measured 6.940 s
execution total; even infinitely fast Watch work could improve that total by
no more than 1.10x, and actual Watch work is only part of the renderer's
`Rendering` stage.

## Before / after summary

| Metric | Pre-#44 v4 | Post-#44 v5 | Current v6 | WatchVm | Interpretation |
|---|---:|---:|---:|---:|---|
| Direct native operation kinds | 75 | 75 | 41 | N/A | v6 routes Lerp and all 36 easing variants through the oracle. |
| VM scalar-cut operation kinds | 86 | 86 | 120 | N/A | The 161-operation inventory remains represented; 41 native + 120 WatchVm cuts. |
| Larp profile dynamic native entries | 134,914 | 34,360 | 34,431 | 0 | v6 is 74.5% below v4; v6 is 0.2% above v5. |
| Larp profile VM/native cuts | 236,089 | 106,039 | 105,410 | 0 | v6 is 55.3% below v4 and 0.6% below v5. |
| Dynamic native operations per entry / allocations and copied bytes | Not captured | Not captured | Not captured | N/A | Static region-size buckets and a profile copied-memory-entry counter exist, but exact dynamic operation distributions and allocation/copy byte totals are not emitted. |
| Callbacks / Watch evaluations / Draws | 1,223 / 2,219,199 / 111 | same | same | same | Exact counts in the Larp time-1 profile. |
| Larp raw Draw SHA-1 | `f06217b8…` | same | same | same | Equality is evidence for this profile, not a proof of all Watch semantics. |
| v4→v5 isolated warm Watch p50, Larp / Baum / ARMAGEDDON | — | 0% / +3.0% / +4.6% GCC time | — | — | The #44 controlled run did not show a general improvement over v4 GCC. |
| Current v6 warm Watch p50, Larp / Baum / ARMAGEDDON | — | — | VM 312/1441/4547 ms; GCC 307/1326/4213 ms | baseline | Exploratory only: host CPU was 43–55% occupied by other work. |
| Cold Larp compiler time | 66.416 s | 27.524 s | 68.986 s, one run | N/A | v4/v5 are #44's measured pair; v6 was measured under different load and is not comparable. |
| Full Larp export | v4/v5 VM/GCC matched in #44 | same | v6 VM/GCC same MP4 hash | same | One pair; GCC 7.274 s vs VM 7.035 s externally, inconclusive. |
| Host runtime ABI calls | 0 | 0 | 0 | 0 | There is no native-to-runtime helper ABI in this implementation. |

The current v4, v5, and v6 CSVs and logs are retained under
`artifacts/sono-gcc-optimization/`. The current v4 matched rerun, v5 rerun,
and v6 run span different periods and host loads. Their absolute times must
not be treated as a controlled v4/v5/v6 comparison. The controlled v4/v5
numbers above are retained from the #44 report; this pass adds v6 execution
and semantic evidence rather than manufacturing a clean timing comparison.

## Execution paths and telemetry

| Path | Result / evidence |
|---|---|
| WatchVm interpreter | Release and test paths remain available. v6 full-chart interpreter export completed; full Rust tests include the deferred GCC precompile test. |
| Legacy/pre-#44 GCC | Archived v4 release executable and cache were rerun for the matched benchmark. There is no selectable legacy-v4 mode in current production source; the archived binary is a comparison artifact, not a shipped path. |
| Optimized GCC | v6 release executable loaded the persistent cache and executed real regions on Larp, Baumkuchen, and ARMAGEDDON. The Larp profile reports selected backend, artifact/cache identity, executed entries, scalar cuts, callbacks, evaluation counts, and Draw hash. |
| Hybrid fallback | Larp profile records ordered WatchVm scalar cuts inside native regions. These are VM-evaluated values; telemetry labels them as cuts rather than claiming they executed natively. |
| Complete fallback | With the compiler deliberately unavailable, `run-watch --vm sono-gcc` exited successfully using WatchVm. It reported zero GCC entries/cuts and matched the interpreter callback/evaluation/Draw results for the Larp checkpoint. |

No host ABI, callback-scale compilation, compiled control flow, or native
memory bypass was introduced by #44. Consequently, ABI helper counts,
per-helper bridge time, and a native-versus-VM callback classification do not
apply. Native regions contain bounded pure scalar operations; memory,
effects, lazy branches, loops, Draw, SFX, particles, and Spawn stay with
WatchVm. Dynamic VM cuts preserve ordered graph evaluation.

The v6 Larp profile had 213,221 graph nodes, 10,104 emitted regions, 26,429
direct-native nodes among 40,368 eligible nodes, and 812 regions using
heap-backed cut inputs. It executed 34,431 regions and 105,410 cuts across
1,223 callbacks. Mean/median/p90/p95/p99 native operations per *dynamic*
entry are not emitted by current telemetry; the size buckets are available
in the saved profile log, but they do not reveal exact sizes inside each
bucket. That distribution statistic remains unverified rather than inferred.
The most frequent remaining VM cuts were `Get` (92,436), `Set` (6,956),
`GetShifted` (4,887), `Lerp` (497), and `If` (412). These are frequency
counts, not per-operation time. The whole-export pipeline instead spent most
of its measured time in preparation and AAC encoding; there is no evidence
that scalar cuts were the dominant end-user bottleneck for this sample.

## Semantic and adversarial verification

### Mismatch found and fixed

The expanded differential test compares exact `f64::to_bits()` values.
It exposed a NaN-payload difference for native `Lerp`; numeric equivalence
would have hidden it. Native `Lerp` and `LerpClamped` were disabled, and all
easing variants—not only Elastic—are now VM cuts. The backend identity changed
from `sono-gcc-codegen-v5` to `sono-gcc-codegen-v6`; this invalidates v5
artifacts under the new operation routing. The targeted Sono-GCC tests and
the named adversarial differential test passed after this change.

The differential corpus includes signed zeros, multiple positive and
negative NaN payloads, infinities, `f64::MIN`/`MAX`, epsilon, smallest
positive and negative subnormals, and ordinary values. The test verifies the
intended native-versus-VM path and exact output bits, Watch evaluation/function
accounting, memory, and display state for the covered numeric operations.
Other current Sono-GCC tests cover representative ordered memory/effect
cuts, lazy inactive branches, dynamic Get addresses, oversized cut input,
Draw/Play/Spawn ordering, and cache coordination.

This is not an exhaustive differential test of every Sonolus operation or
all Prompt #45 adversarial scenarios. In particular, the pass did not add
an exhaustive generated corpus for every loop/branch/switch/invalid-memory
case, all entity lifecycle permutations, or every operation under both
sequential and parallel callback scheduling. The VM remains the fallback for
those semantics.

### Real-engine/profile and whole-chart parity

- **Larp 64x:** time-1 interpreter/GCC profile Draw hash `f06217b8eafd30644a7f2a36ae0c94e4e4adf4df`, 1,223 callbacks, 2,219,199 evaluations, 111 Draws. Full v6 whole-chart interpreter and GCC exports are byte-identical at 459,559 bytes and SHA-256 shown above. FFprobe reports 16×16, 1 fps, 20 s H.264 video and AAC audio. Both report SFX hash `065623858bbffa464f9c2da960afbeefd8d4586b`, 93 scheduled effects, 3 loop starts, and 3 loop stops.
- **Baumkuchen:** time-1 interpreter/GCC Draw hash `01205b0a457cbedd7e2337408acf473355479cd8`; current repeated v6 benchmark exited successfully in both modes.
- **ARMAGEDDON:** time-1 interpreter/GCC Draw hash `95651773e964b5fb8504abdeb902af34a70d4c5f`; current repeated v6 benchmark exited successfully in both modes.
- **Dreamer:** no new #45 execution. It is not present in the current local level fixture set for this pass.
- **Previously unseen graph:** not verified. All real graphs used here were earlier project/benchmark fixtures; no valid engine graph known to be unseen by #44 tuning was available.

No gameplay screenshot or exported frame was visually inspected in this compiler audit. The MP4 comparison verifies deterministic bytes and streams, not visual quality. No separate particle sequence hash or final audio PCM hash was captured; SFX event scheduling and the complete encoded MP4 hash matched for the Larp whole-chart pair.

## Performance methodology and results

The release benchmarks use the repository's
`scripts/benchmark_sono_gcc.ps1`, the same engine, LevelData, resources, and
release mode for both execution modes, five fresh processes per mode, and
`--no-vm-accounting`. Current v4/v5/v6 repetitions are saved as individual
CSV rows and full logs. Workloads were the real Larp 64x, Baumkuchen, and
ARMAGEDDON charts using Next RUSH Official and an identical resource archive
(SHA-256 `BF6D73A790D812312BEE72B8F7EF07B6D010E01E476E0BF390C9A2543AD4BF58`).

The v6 persistent-cache warm medians and observed ranges were:

| Chart | VM median [min–max] | GCC median [min–max] | GCC vs VM median |
|---|---:|---:|---:|
| Larp 64x | 312 [281–401] ms | 307 [272–329] ms | −1.6% |
| Baumkuchen | 1,441 [1,385–1,667] ms | 1,326 [1,236–1,481] ms | −8.0% |
| ARMAGEDDON | 4,547 [4,426–4,646] ms | 4,213 [4,045–4,274] ms | −7.3% |

These numbers are **exploratory, not a speed claim**: during the v6 run the
host was under 43–55% measured CPU load and other large workloads were
running. They show GCC did not catastrophically regress in that sample, but
do not meet the prompt's clean-host performance bar. The v6 Larp persistent
cache observations were 288 ms lookup, 11 ms DLL load, and 0 ms compile in
the profiling run; profiling itself reported a 450 ms Watch frame and is
not used as normal timing evidence.

The isolated v4→v5 release comparison in the #44 report used five runs per
mode. GCC medians changed by 0% on Larp, +3.0% (slower) on Baumkuchen, and
+4.6% (slower) on ARMAGEDDON. GCC remained faster than VM in the v5 medians
for those one-frame workloads. This does not demonstrate that singleton
filtering improved steady-state GCC runtime versus legacy GCC. It did reduce
the recorded Larp crossing counts and the reported cold compile time in that
controlled pass.

### Cold compile and pipeline / Amdahl

- #44's matched cold Larp compile observations: v4 66.416 s; v5 27.524 s.
- v6 cold Larp compile: 68.986 s, one observation on the loaded host; not
  compared as a regression because environment/load differed.
- v6 full-chart GCC internal execution: 6.940 s total; `Preparing` 4,546 ms,
  `AudioMixing` 103 ms, `AudioEncoding` 1,492 ms, `Rendering` 646 ms, and
  `Finalizing` 151 ms. Stage sums can differ from the total due measurement
  boundaries and concurrency. The `Rendering` stage is a conservative upper
  bound for Watch work, so the theoretical speedup ceiling for this export
  is at most 6.940 / (6.940 − 0.646) = **1.10x**. WatchVm/GCC wall observations
  for the complete export were 7.035/7.274 s respectively; this one pair is
  inconclusive and shows no measured end-user improvement.

The result is therefore not “faster rendering.” The supportable conclusions
are reduced Larp dynamic entry/cut counts relative to v4 and a v5 cold compile
reduction in the earlier matched run. The current data does not prove a
reliable whole-render speedup.

## Cache, fallback, GUI, and resource audit

- **Cache identity:** the compiler backend version is now v6. Existing cache
  manifest validation and cache rejection tests remain in place; the full
  suite exercises them. The codegen identity prevents v5 native output from
  being selected for the v6 routing policy.
- **Fallback:** deliberately unavailable Rust compiler produced a successful
  Larp WatchVm run with matching checkpoint output. This pass did not manually
  corrupt each DLL/metadata variant, remove every symbol, or cancel a real
  rustc compile. Unit tests cover malformed/stale cache manifests and
  concurrent same-graph compile coordination.
- **GUI precompile:** the full test suite includes
  `deferred_gcc_selection_uses_vm_when_background_program_is_not_ready` and
  `concurrent_precompile_requests_share_one_safe_exact_graph_compile`.
  Interactive GUI timing, rapid engine switches, and closing the application
  during real compilation were not exercised.
- **Interpreter compatibility:** the complete Larp VM render works with the
  same project inputs; the test suite validates the immediate VM-on-miss
  behavior. No project requires GCC selection.
- **Resources:** this pass did not perform a long repeated engine-switch
  soak or measure process working-set/handle/DLL counts. No leak conclusion
  is made. Existing cache bounds and compilation coordination tests passed
  as part of the suite.

## Claim audit and recommended status

The #44 report already says no general steady-state runtime win; that
qualification remains correct. The implementation report and coverage
inventory now point to the v6 policy. Documentation should not advertise
“faster” or a percentage runtime speedup based only on fewer calls, lower
region count, or compilation time. The precise supported claim is:

> In a profiled Larp 64x Watch frame, Sono-GCC v6 executed 34,431 native
> regions and 105,410 VM-to-native scalar cuts, versus the archived v4
> profile's 134,914 entries and 236,089 cuts; full-chart v6 VM and GCC MP4s
> were byte-identical. A general whole-render speedup was not established.

**Classification: C, workload-dependent (provisional).** GCC's v6 repeated
warm medians were lower than VM on the three sampled charts, but the host was
busy; the complete Larp export timing went the other way in one pair; and
the controlled v4-to-v5 comparison showed no GCC improvement on Larp and
small regressions on the other two charts. Thus category A/B is not
substantiated. Category D is also too strong because the current repeated
sample did not show VM faster on all isolated workloads. The more useful
conclusion is that the effect depends on what is measured and needs clean
benchmark reproduction.

**Default recommendation: experimental opt-in.** Do not make Sono-GCC the
default or auto-select it from these data. WatchVm remains the default and
oracle. The current compiler remains Windows-only and requires an available
Rust compiler for a cold compile; safe VM fallback remains available.

## Prompt #45 requirement matrix

| Area | Status | Evidence / limitation |
|---|---|---|
| Real optimized path and telemetry | Verified | Release v6 profile and real charts show actual entries/cuts and artifact/cache selection. |
| Crossing reduction | Verified for Larp profile | v6 vs archived v4 profile: −55.3% cuts; metric is same ordered VM-cut counter. |
| Native granularity | Partially verified | Dynamic size buckets recorded; exact dynamic mean/median/p90/p95/p99 not emitted. |
| Host ABI | Not applicable | No host runtime ABI or native-to-VM callbacks exist in current code. |
| Semantic differential | Tested, bounded | Exact-bit adversarial numeric set; representative effects/memory/lazy ordering tests. Not exhaustive against all listed edge cases. |
| Real engines | Tested | Larp, Baumkuchen, ARMAGEDDON checkpoint runs. Dreamer absent locally. |
| Whole chart and SFX | Tested | Larp v6 VM/GCC MP4 hash, H.264/AAC metadata, SFX hash/counts match. One low-resolution fixture. |
| Unseen graph | Unavailable | No suitable graph unseen by #44 tuning was available. |
| Repeated performance | Exploratory only | Five runs/mode/fixture, but measured host was loaded. No clean release-grade conclusion. |
| Cold vs warm cache | Partly measured | v6 one cold compile; five warm process runs; persistent cache profile. Cold comparison across versions is contaminated. |
| Fallback and cache | Tested | Forced unavailable-compiler fallback; cache identity and rejection/coordination tests. Not every failure mode induced manually. |
| GUI precompile | Headless-tested | VM-on-miss and concurrent compile tests; no interactive GUI run. |
| Memory/concurrency soak | Not tested | No repeated-process memory/handle soak or shutdown race certification in this pass. |
| Visual output inspection | Not performed | Deterministic hashes and FFprobe only; no frame judged visually. |

## Build and test certification

- `cargo fmt --all -- --check`: passed.
- `cargo check --offline`: passed.
- `cargo clippy --offline --all-targets`: exited successfully. Clippy emitted
  numerous repository warnings (including 195 warnings in the library test
  target); no warning gate is configured.
- `cargo build --release --offline`: passed for the v6 executable, SHA-256
  `4BBD755F789A2CD01BE1FBF48405B629C666588EE8FCBD7FA31CEA9FE85CBFBF`.
  The build reported `rc.exe unavailable`, so Windows executable version
  metadata was not embedded.
- `cargo test --offline`: passed: 419 library tests, 27 binary/CLI tests,
  101 renderer integration tests, 9 scheduler tests, and the other enabled
  targets; 8 tests were ignored because they require optional local tools or
  fixture provisioning. No test failure or abnormal harness abort occurred
  in this run. Tests that require app-discoverable FFmpeg and interactive GUI
  or real hardware remain outside this evidence.
- A Windows release executable ran real Watch/GCC workloads and completed
  the whole-chart parity exports. The validation bundle's FFmpeg/FFprobe were
  available from a local package hard link; this is not a clean-machine or
  packaged-installer test.

## Reproduction

Use PowerShell from the repository root. The exact release executables,
engine/resource copies, logs, and CSVs used here are in the local artifacts
tree (not required by the application):

```powershell
cargo test --offline sono_gcc::tests
cargo test --offline sono_gcc::tests::native_regions_and_vm_cuts_match_watch_vm_over_adversarial_numbers
```

The benchmark script's recorded v4, v5, and v6 CSVs are under:

```text
artifacts/sono-gcc-optimization/verification-v4-matched/
artifacts/sono-gcc-optimization/verification-v5/
artifacts/sono-gcc-optimization/verification-v6-warm/
```

The whole-chart logs/videos are under
`artifacts/sono-gcc-optimization/whole-chart/`. Their FFmpeg/FFprobe binaries
were hard-linked from a locally available validated FFmpeg 8.1 dependency
into the isolated validation `UserData`; no network download was used.

Example repeated v6 benchmark invocation (use the same local fixture paths
and the archived v4 executable to reproduce the corresponding baseline):

```powershell
.\scripts\benchmark_sono_gcc.ps1 `
  -Name verification-v6-larp64x `
  -Engine 'C:\Users\Admin\Desktop\Charts\Next RUSH Official' `
  -Level 'C:\Users\Admin\Desktop\Charts\In Progress\larp 64x\larp 64x.json.gz' `
  -Resources 'artifacts\sono-gcc-optimization\verification-v6\benchmark-resources.scp' `
  -Renderer 'artifacts\sono-gcc-optimization\verification-v6\renderer.exe' `
  -Runs 5 -NoVmAccounting `
  -OutputDirectory 'artifacts\sono-gcc-optimization\rerun-v6'
```

## Final finding

The optimization is real in its execution path and reduces Larp's dynamic
crossings versus v4. It has not demonstrated a dependable whole-render
speedup, and the v4→v5 data does not show a steady-state GCC improvement over
legacy GCC. The v6 semantic correction properly routes payload-sensitive
operations through WatchVm. Keep the compiler opt-in, preserve the
interpreter as the oracle, and do not claim general acceleration until a
clean, randomized, repeated release benchmark and broader semantic fixture
set support it.
