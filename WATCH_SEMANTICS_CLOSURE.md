# Post-#48 Watch Semantics Closure

**Scope:** close verified Sonolus Watch runtime defects and expose any remaining unsupported semantics without changing unrelated product areas. **One WatchVm remains authoritative; Sono-GCC continues to cut through it or fall back to it.**

## Baseline and inspected evidence

- Current checkout was already dirty before this task. No reset, cleanup, or commit was performed.
- Focused pre-change baseline: `cargo test --locked --test m0 watch_vm -- --test-threads=1` passed **38 tests**, with 63 filtered out.
- The current `POST_48_RELEASE_CLOSURE.md` records final serial and parallel all-target baselines of **580 passed, 8 ignored, 0 failed**. Those are historical evidence from the same post-#48 checkout; this task will rerun final validation after any production change.
- Inspected `src/runtime.rs`, `src/watch_runtime.rs`, `src/offline.rs`, `docs/SONOLUS_V1_1_4_CAPABILITY_AUDIT.md`, `docs/SONOLUS_V1_1_4_ORACLE_ARITY_AUDIT.md`, `docs/KNOWN_LIMITATIONS.md`, `POST_48_RELEASE_CLOSURE.md`, `RELEASE_CANDIDATE_CERTIFICATION.md`, `SONO_RENDERER_FOLLOWUP_48_CONVERGENCE.md`, `FEATURE_EXISTENCE_AUDIT.md`, `SONO_GCC_OPTIMIZATION_VERIFICATION.md`, `SONO_GCC_COVERAGE.md`, current `tests/m0.rs`, and oracle-packet `RESULT.md` files.
- The VM already implements the scalar arithmetic, comparisons, control flow, memory access, timing conversion, stream queries, resource queries, draw-quad command, particle instance commands, spawn requests, and backend-neutral audio events described in the current capability audit. Passing local tests establishes Sono-Renderer behavior, not complete Sonolus conformance.
- The oracle packet contains partial client recordings. Historical Random and While/Break crashes are invalid semantic evidence because their packages had malformed arities. The corrected fixtures have not been client-run. Partial stack, StreamSet, lifecycle, and particle observations do not establish the unresolved edge behavior.
- Official references inspected: [Stack Functions](https://wiki.sonolus.com/engine-specs/functions/stack-functions), [Replay Data](https://wiki.sonolus.com/replay-specs/resources/replay-data), [StreamSet](https://wiki.sonolus.com/engine-specs/functions/stream-set), [DrawCurvedB](https://wiki.sonolus.com/engine-specs/functions/draw-curved-b), [Random](https://wiki.sonolus.com/engine-specs/functions/random), and [Sonolus 0.7.0 release notes](https://wiki.sonolus.com/release-notes/versions/0.7.0). The stack overview explicitly places stack storage at the end of Temporary Memory and points implementers to external stack literature; this does not define the target client's exact pointer/frame/growth edge rules.

## Gap register

Status before work is one of **IMPLEMENTED_UNVERIFIED**, **PARTIAL**, **STILL MISSING**, **SPECIFICATION AMBIGUITY**, **BLOCKED BY ENVIRONMENT**, or **OUT OF V1 SCOPE**. A local unit test is not promoted to a client-conformance result.

| ID | Area and expected behavior/evidence | Current implementation and discrepancy | Affected engines/fixtures; severity | Reproduction and proposed correction | Regression evidence required | Status before work |
|---|---|---|---|---|---|---|
| W01 | Scalar operation set must follow each v1.1.4 function's documented operand order and numeric behavior. Official function pages plus exact client/reference observations are evidence. | `WatchVm` implements the supported operations. Exact exceptional-float parity (signed zero, NaN payloads, subnormals, overflow, division/remainder, libm corners) is not fully client-checked. No specific current wrong result was established by the audit. | Shared by all Watch engines; P3 unless a mismatch is demonstrated. | Run isolated valid-arity operation probes against v1.1.4 and compare `to_bits`; correct only demonstrated mismatches. | Exact-bit unit tests for every confirmed mismatch plus one valid fixture/client transcript. | IMPLEMENTED_UNVERIFIED / validation gap |
| W02 | Control flow must evaluate only selected/lazy branches, propagate `Break(count,value)` through the requested block depth, and honor the versioned While/DoWhile contract. Official signatures and release history conflict on While versioning. | `If`, `Switch*`, `JumpLoop`, `Block`, `Break`, `DoWhile`, and one-/two-child `While` are implemented and have focused tests. One-argument While remains local compatibility; two-argument form is exercised by supplied engines. No valid client observation settles v1.1.4 While shape or all break edges. | Exorcist, Horizon/Dreamer, Next SEKAI and engines using loops; P0 if wrong, currently P3. | Compare isolated, signature-valid fixtures for selected branch, nested break counts, loop result, and exact v1.1.4 While arity. Preserve local legacy behavior unless a valid target-client result justifies change. | Lazy side-effect, nested break, termination, and version-specific fixture tests. | IMPLEMENTED_UNVERIFIED / SPECIFICATION AMBIGUITY |
| W03 | Stack operations must implement stack storage in Temporary Memory and all documented operations with target-compatible pointers, frames, and growth. Official stack overview and per-operation pages give names/signatures; overview defers layout details to external literature. | All 14 `Stack*` functions are rejected as unsupported. Client recording shows only an incomplete, quickly-scrolled subset; it cannot identify pointer direction, offsets, frame contents, return values, capacity, or entity isolation. | Any engine compiler/runtime using stack functions; P2 compatibility blocker. | Capture the existing valid `oracle-stack` fixture as a complete ordered transcript, including fresh entity state, overflow/underflow, nested frames, and `StackGrow`. Implement only after the target behavior is sufficiently distinguished. | Test every op, nested frame, pointer restore, memory overlap, bounds, and per-entity isolation against the client transcript. | STILL MISSING; details SPECIFICATION AMBIGUITY |
| W04 | Random functions must produce target-compatible values and maintain the correct random state/seed scope. Official [Random](https://wiki.sonolus.com/engine-specs/functions/random) reference defines the two-argument shape, but the algorithm/seed lifecycle is not established by the current evidence. | `Random` and `RandomInteger` are unsupported. The historical crash fixtures used invalid `Random()` arity; valid regenerated fixtures are unrun. No output-range, sequence, seed, or callback-lifecycle observation exists. | Any engine invoking random functions; P2 compatibility blocker. | Run valid two-argument probes for bounds, repeated calls, entities, callbacks, and render restart through the current Sonolus client. Do not choose a PRNG from the operation names. | Exact result sequence/range tests tied to a recorded client version and deterministic seed behavior if the client exposes one. | STILL MISSING / BLOCKED BY CLIENT ORACLE |
| W05 | Replay streams must be attached to Watch from Replay Data and queried with correct decoded keys/values. Official replay schema defines optional `{id, keys, values}` streams; keys and values are delta encoded. | Optional bounded Replay Data decoding and attachment now exist on `WatchRuntime`, `WatchFrameStepper`, and `FrameSession`. The ordinary render configuration remains replay-optional. | Replay Watch workflows; P2 compatibility. | Decode the official gzip JSON schema; validate pairs/order/finiteness/entry count; inject before preprocessing. | Decoder, malformed-input, interpolation/query, and late-attachment tests pass. No real attached-client replay comparison was available. | IMPLEMENTED BUT NOT VALIDATED against a real client |
| W06 | `StreamSet(id,key,value)` must set state with documented return value and compatible collision/visibility/lifetime semantics. Official page supplies signature/summary; client fixture must determine interactions not stated there. | Function is unsupported. Existing valid fixture reached callbacks, but the visible reader row does not establish same-key replacement, sequential/parallel visibility, replay interaction, or reset lifetime. | Engines using mutable streams; P2 compatibility blocker. | Capture complete writer/read rows in controlled sequential order and parallel callbacks; distinguish replay stream from mutable output state. Implement only after the state model is evidenced. | Same-key replacement, missing key, cross-callback, callback/frame reset, replay-read and parallel determinism tests. | STILL MISSING / SPECIFICATION AMBIGUITY |
| W07 | Six `DrawCurved*` operations must produce the specified curved sprite geometry and preserve display-list order/resources; raster output must match graphics rules. | All six operations now dispatch through the existing VM and emit segmented quads with per-patch source UVs. CPU and GPU render paths map the same UV rectangles. | Engines using curved draw; P1 visible output / P2 compatibility. | Use the official quadratic control-point/edge/segment contract. Exact client tessellation, shared-edge coverage, and render-mode parity remain unobserved. | VM dispatch and geometry tests, CPU source-crop test, GPU source-crop test pass. No real-client pixel reference was available. | IMPLEMENTED BUT NOT VALIDATED against client pixels |
| W08 | Standard and Lightweight render modes must apply their documented interpolation/raster rules. | Existing display-list/render path has partial mode support; the audit has no equivalent native-resolution pixel pair proving parity or intentional difference. | Engines selecting either mode; P1. | Capture matched client frames and compare renderer output; fix mode-specific geometry or sampling only from evidence. | Pixel references across axis-aligned, rotated, perspective, and curved quads. | PARTIAL / BLOCKED BY PIXEL ORACLE |
| W09 | Runtime lifecycle must preserve documented preprocessing, spawn/updateSpawn, initialize, sequential/parallel update, terminate, frame persistence, and seek reconstruction. | `WatchRuntime` is the sole host and has source-order scheduling, memory views, and seek logic. Client packet confirms only limited A sequential/parallel calls and one Entity Info slot; it lacks complete boundaries, dynamic Spawn, termination, and seek transcript. No concrete local defect established. | All engines; P0 if state corrupts, currently P3 validation. | Run timestamped lifecycle fixture over `0→1→0→1` and immediately around lifetime boundaries, including dynamic spawn with a documented archetype identifier. Compare state/callback records. | Deterministic serial/parallel lifecycle and seek checkpoint tests against valid client observations. | IMPLEMENTED_UNVERIFIED / validation gap |
| W10 | Memory blocks must honor initialization, lifetime, aliasing, entity scope, and read/write permissions for every callback. | Runtime provides block backing and lifecycle permissions with focused tests and real-engine fixture use. Full block × callback permission and seek-reset matrix is incomplete; no specific corruption defect was reproduced. | All engines; P0 if state aliases/corrupts; currently P3 validation. | Add table-driven valid read/write attempts across blocks and callbacks, including adjacent entity rows and backward seek. Compare to official block access tables. | Permission matrix, alias/bounds, persistence-across-callback and seek tests. | PARTIAL / validation gap |
| W11 | BPM/time/timescale conversion must preserve Watch/media mapping, segment boundaries, negative/zero timescale, and start anchors. | `VmContext` implements conversion maps; existing tests and representative engines cover subsets. No complete client matrix establishes unusual boundaries or all zero/negative timescale behavior. | Engines with BPM/time changes; P1 timing/audio. | Exercise authored segment boundaries and negative/zero rates in isolated fixtures and compare exact function results/client timing. Preserve established `media time = Watch time + bgmOffset`. | Exact boundary and inverse/round-trip tests plus representative client transcript. | IMPLEMENTED_UNVERIFIED / validation gap |
| W12 | Skin transforms, Draw quads, resource aliases, and display-list ordering must follow Watch blocks and graphics rules. | Core Draw quad path, skin/resource presence, runtime background, and resource binding exist; curved geometry and full Standard/Lightweight raster parity are tracked in W07/W08. No new ordinary-quad defect was found. | All visual engines; P1. | Run a captured ordinary quad/transform reference with overlapping sprites and resource aliases. | Ordering, transform, alpha, resource binding, and pixel parity tests. | IMPLEMENTED_UNVERIFIED; curved subset STILL MISSING |
| W13 | Particle Spawn/Move/Destroy must preserve valid handle state and produce the expected visual instances; invalid/repeated handle behavior must match the target. | VM keeps shared instance state and emits commands; Watch consumes state/events. Partial client recording shows Spawn returned `0` and callbacks continued, but cannot tell whether invalid/repeated operations mutate hidden state or establish visual timing/raster behavior. | Engines using particles; P1 visuals. | Capture before/after each operation at fixed time/viewport with valid/invalid/repeated handles and loop/duration variants. Separate VM event assertions from raster sampling. | Handle-state tests and particle pixel/time progression references. | PARTIAL / SPECIFICATION AMBIGUITY |
| W14 | Audio operations must emit/schedule correct Watch-time events, loop handles, stops, and offsets; a downstream audio backend is a separate requirement. | VM creates backend-neutral commands/events; tests cover command order, loop handles and scheduled timing. No authoritative audible client comparison or Watch effect playback/mixing backend is present. | Engines with SFX; P1 audible output. | Compare a valid client audio fixture's event time/id/loop behavior. Keep a backend absence classified separately from VM semantic correctness. | Event exactness tests and an audible client/reference comparison; backend integration test if claimed. | PARTIAL; backend playback remains missing |
| W15 | Resource checks and resource payload identity must reflect the loaded engine bundle. | Resource IDs/names, `Has*`, and engine-resource loading are implemented. Audit reports no current defect; resource-complete real-engine runs exist for selected fixtures, not every asset kind. | Engines with custom resources; P2/P3. | Run selected package with present, absent, and alias resources. | Loader identity plus `Has*` and render reference tests. | IMPLEMENTED_SPEC / incomplete validation |
| W16 | Sono-GCC must preserve the corrected VM semantics and must fall back safely when it cannot. | GCC remains optional; runtime identity/versioning, scalar WatchVm cuts, and VM fallback are existing architecture. #48 did not run a new adversarial differential suite for Watch gaps. | Only opt-in GCC-eligible Watch operations; P0 if parity breaks. | After each semantic implementation, rerun exact-bit adversarial differential tests; prevent unsupported native assumptions by routing through VM. | Existing exact-bit/cut/fallback tests plus final adversarial suite. | IMPLEMENTED but task validation required |
| W17 | Valid Watch function arities should be enforced or deliberately accepted by the versioned contract. | Existing arity audit corrects invalid historical fixtures; some operations, e.g. `Get`/`Set`, consume leading operands without explicit over-arity rejection. One/two-argument While policy is called out separately in W02. No observed mismatch proves extra children are invalid in runtime graphs. | Any engine with malformed/noncanonical graph; P3. | Compare signature table to version-pinned public function list; inspect whether client ignores surplus serialized children. Do not tighten arity as a cleanup-only change. | Table-driven accepted/rejected arities for documented fixed and variadic operations. | SPECIFICATION AMBIGUITY / validation gap |

## Implementation decision before production edits

The preceding evidence does not support inventing stack layout, a PRNG sequence/seed policy, mutable stream collision/visibility semantics, particle edge behavior, or exact client callback ordering. Those are genuine missing behaviors, but currently require a valid, complete client observation or a more explicit versioned specification. The six curved-draw pages explicitly define their quadratic control-point construction, segment count, and edge orientation; a generic tessellated mesh can therefore be implemented from that contract, while pixel and render-mode parity remain separate validation gates. Replay's official schema explicitly defines gzip JSON and delta-encoded stream arrays, so an optional bounded WatchRuntime replay-stream input can be implemented and tested without changing non-replay renders. Unresolved client-dependent details remain blockers rather than being simulated from operation names.

## Final status by gap

| ID | Final classification | Evidence / remaining work |
|---|---|---|
| W01 | IMPLEMENTED BUT NOT VALIDATED | Existing VM tests pass; no complete target-client exceptional-float bit matrix. |
| W02 | SPECIFICATION AMBIGUITY | Lazy branch, loop, break, and supplied-engine tests pass; exact v1.1.4 While arity/client break transcript remains unresolved. |
| W03 | STILL MISSING | Fourteen stack operations remain unsupported. Partial captures do not define stack layout or lifecycle. |
| W04 | STILL MISSING / BLOCKED BY CLIENT ORACLE | `Random` and `RandomInteger` remain unsupported; no valid client sequence/seed evidence. |
| W05 | IMPLEMENTED BUT NOT VALIDATED | Bounded decoder and runtime/offline injection are locally tested; real attached-client replay comparison and CLI replay selection remain open. |
| W06 | STILL MISSING / SPECIFICATION AMBIGUITY | `StreamSet` remains unsupported because collision, visibility, lifetime, and parallel semantics are not established. |
| W07 | IMPLEMENTED BUT NOT VALIDATED | All six VM operations, patch geometry, CPU/GPU source UV mapping tested; real-client pixels and both render modes remain open. |
| W08 | BLOCKED BY CLIENT PIXEL ORACLE | Existing render-mode tests pass, but no matched client pixel reference, especially for curved quads. |
| W09 | IMPLEMENTED BUT NOT VALIDATED | Existing lifecycle/seek and real-engine tests pass; exhaustive target-client ordering transcript is absent. |
| W10 | IMPLEMENTED BUT NOT VALIDATED | Permission, memory, bounds, and lifecycle tests pass; the full client callback/block matrix remains open. |
| W11 | IMPLEMENTED BUT NOT VALIDATED | Existing timing, BPM, and timescale tests pass; unusual boundary and client comparison matrix remains incomplete. |
| W12 | IMPLEMENTED BUT NOT VALIDATED | Ordinary Draw and render regressions pass; curved subset now has VM/CPU/GPU implementation and local coverage, but lacks client pixel parity. |
| W13 | SPECIFICATION AMBIGUITY | Particle command/state tests pass; invalid/repeated handle and visual simulation semantics remain unresolved. |
| W14 | PARTIAL / IMPLEMENTED BUT NOT VALIDATED | VM event/scheduling tests pass. An audible Sonolus effect playback backend and client audio comparison remain absent. |
| W15 | IMPLEMENTED BUT NOT VALIDATED | Resource loader and availability tests pass for selected fixtures; not every resource/engine package was validated. |
| W16 | FIXED AND VERIFIED for tested paths | Existing adversarial VM/GCC differential and fallback tests passed. New curved operations remain WatchVm-only; no native coverage was added. |
| W17 | SPECIFICATION AMBIGUITY | Arity audit corrected malformed oracle fixtures; no new strictness was added without versioned evidence. |

## Changes implemented in this closure

- Added `WatchReplayStreams`, a gzip JSON decoder for the official optional Replay Data `streams` field. It accumulates delta-encoded keys and values, rejects malformed lengths, duplicate IDs, non-finite values, unordered keys, overflows, decompression above 64 MiB, and more than 1,000,000 total entries. Decoded events are moved into the authoritative VM context rather than copied.
- Added pre-execution replay attachment to `WatchRuntime`, `WatchFrameStepper`, and `FrameSession`. Replay is optional and does not change ordinary renders.
- Implemented all six `DrawCurved*` dispatch operations in the existing `WatchVm`. Curved quadratic edges are tessellated into ordered patches. Each patch stores its normalized source-atlas UV rectangle; CPU and GPU sampling apply that rectangle consistently.
- Preserved ordinary `SpriteDraw` JSON compatibility: missing source UV decodes as the full sprite, and the full default is omitted when serialized. Ordinary draw hashes retain their historical identity; non-default UV crops participate in the hash.
- Updated the operation capability classifier, compatibility wording, capability audit, and known limitations to reflect the implementation while keeping client-parity limits explicit.

## Regression tests added

- Replay schema cumulative delta decoding and Watch query integration.
- Replay malformed pair, duplicate ID, decreasing key, overflow, entry-limit, and late-attachment rejection.
- All six curved operation patch generation, straight/bowed edges, source UV coverage, VM dispatch, and legacy draw serialization.
- CPU and GPU source-UV crop sampling against a four-color texture.

## Final validation

Validation ran after the last production-code change:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed; Cargo warned it could not canonicalize `C:\Users\Admin`. |
| `cargo check --locked --all-targets` | Passed. Build script warned `rc.exe` was unavailable, so Windows executable version metadata is not embedded. |
| `cargo clippy --locked --all-targets` | Passed with the repository's existing warning set (199 warnings, no errors). |
| Full serial all-target suite | Passed: library 449 passed / 8 ignored; CLI 27; `arctan2` 3; `m0` 103; scheduler 9. Total **591 passed, 8 ignored, 0 failed**. |
| Full parallel all-target suite | Passed with the same **591 passed, 8 ignored, 0 failed**. No heap-corruption, access-violation, or harness-abort signature was observed. This does not establish the historical Windows abort is fixed. |
| Focused replay, curved-draw, capability classifier tests | Passed, including decoder, VM dispatch, geometry, CPU, and GPU sampling. |
| Existing adversarial Sono-GCC/VM differential tests | Passed as part of both complete suites. No native eligibility expansion was made. |
| Real-engine fixtures | Existing Baumkuchen, ARMAGEDDON, Next RUSH, Horizon/Dreamer, and other fixture tests in the full suites passed. These runs do not contain an attached Replay Data oracle or curved-client pixel reference. |
| Ignored real-media tests run explicitly | Passed: standalone video/audio MP4, template reuse/portrait/gameplay, subtitle extraction, filter-file lifetime, and queued project-to-fixture render. |
| `cargo build --locked --release` | Passed. `rc.exe` warning remains. |
| Performance Lab comparison | Not run. This pass added no native Watch optimization; the GPU shader now maps UVs for draws and the new curved path has no real-engine workload baseline. Do not infer a speedup or performance parity. |
| Client-oracle tests | Not available: no valid target-client stack/random/StreamSet transcripts or labeled curved/replay pixel/audio references were run in this environment. |

## Remaining implementation and validation gaps

- **Stack:** all fourteen operations remain unsupported. Required next step: capture the valid `oracle-stack` fixture with a complete, ordered v1.1.4 client transcript covering pointer/frame/growth, underflow/overflow, nesting, and entity scope; then implement the observed contract.
- **Random:** `Random` and `RandomInteger` remain unsupported. Required next step: run valid two-argument fixtures in the current client to determine bounds, algorithm, seed scope, and callback/restart behavior.
- **Mutable streams:** `StreamSet` remains unsupported. Required next step: record controlled sequential and parallel same-key, cross-callback, frame-reset, replay-read, and return-value observations.
- **Curved rendering:** the documented geometry is implemented, but exact client segment boundaries/coverage and Standard/Lightweight pixel parity are not proven. Required next step: capture labeled, native-resolution outputs with a nonuniform texture for all six ops in both modes; compare preview/export and edge pixels.
- **Replay:** local decoder/runtime integration is tested; a real Replay Data client comparison and top-level CLI replay-file selection are still absent.
- **Other audited gaps:** exceptional float/client parity, exact While/break version contract, exhaustive memory permissions/timing boundaries, particle edge/visual semantics, and audible SFX playback remain partially validated or absent as documented above.
- **Performance:** no comparable Performance Lab measurement was available before and after; report no performance conclusion.
- **Windows instability:** both suites were clean, but there is still no root cause. Continue monitoring rather than closing that issue.

## Overall decision

The verified replay host gap is closed locally, and curved Draw now has a generic implementation path. Stack, random, and mutable `StreamSet` are still missing, and curved/replay parity against a valid Sonolus client remains unverified. Therefore the required Watch semantic surface is not fully closed.

**WATCH SEMANTICS CLOSURE INCOMPLETE**

## Continuation B — remaining operation families (2026-10-08)

Continuation B re-inspected the current `WatchVm`, `WatchRuntime`, replay stream map, operation classifier, Sono-GCC inventory, oracle fixtures, and the post-#48 evidence registers before deciding whether production behavior could be added safely. No production code was changed in this continuation: the available specification and client evidence still do not define enough behavior to implement these families without guessing.

| Family | What the versioned public contract establishes | What remains unestablished | Current result |
|---|---|---|---|
| Stack (14 operations) | The functions are compiler stack helpers stored at the end of Temporary Memory. The public Temporary Memory block documentation gives indices 0–4095 and warns that initial values are unpredictable. Individual pages specify several signatures and return descriptions. | The stack overview defers implementation details to external literature. The current client capture is incomplete and cannot establish pointer direction/base, frame layout, grow/leave behavior, invalid pointers, overflow/underflow, or callback/entity lifetime. | **UNSUPPORTED / SPECIFICATION AMBIGUITY.** A fixed 4096-slot block bound is known; the stack ABI is not. Do not infer the missing ABI from a conventional stack. |
| `Random`, `RandomInteger` | [`Random(min,max)`](https://wiki.sonolus.com/engine-specs/functions/random) returns within a closed interval; [`RandomInteger(min,max)`](https://wiki.sonolus.com/engine-specs/functions/random-integer) returns integers in a half-open interval. | The public pages do not define generator algorithm, initialization/seed, state ownership, advancement, or cross-callback/entity behavior. The corrected valid-arity fixture files exist but have no valid client transcript. | **UNSUPPORTED / BLOCKED BY CLIENT ORACLE.** Range contract is documented; sequence and reproducibility contract are not. No arbitrary PRNG was added. |
| `StreamSet` | [`StreamSet(id,key,value)`](https://wiki.sonolus.com/engine-specs/functions/stream-set) sets an entry and returns `0`; the existing stream query functions read ordered key/value series. | The public entry description does not specify duplicate-key replacement, ordering for non-finite values, callback visibility/lifetime, parallel writes, replay interaction, or how mutation is reset during frame reconstruction. Existing client video is partial. | **UNSUPPORTED / SPECIFICATION AMBIGUITY.** The basic signature/return are known; state semantics needed by the runtime are not. |

The valid generated probes are available under `artifacts/real-sonolus-oracles/watch-runtime-blockers/generated/local/` and their source/runbook remains in the oracle packet. This environment has no `Sonolus` executable on `PATH`; no target client was available to run them. The existing partial recordings do not contain enough labeled output to serve as a replacement oracle. The local Watch fixture names those operations but currently fails at the unsupported VM dispatch, so such a run would not count as real-engine semantic validation.

Sono-GCC remains unchanged: none of these stateful/unsupported operation families is direct-native or newly eligible. If later implemented, they must remain WatchVm cuts unless exact native equivalence and ordering safety are separately proven. The previous replay decoder and curved Draw implementation are also unchanged; no new client-parity claim is made.

**Continuation B status: WATCH SEMANTICS CLOSURE INCOMPLETE.** Next required evidence is a complete v1.1.4 stack transcript, valid two-argument random probes with recorded repeat/callback observations, and complete sequential/parallel StreamSet writer-reader rows (including replay attachment). Until that evidence or a more explicit versioned contract exists, adding production behavior would violate the prompt's no-guessing rule.

## Pragmatic compatibility closure (2026-10-08)

The later pragmatic closure explicitly authorizes bounded approximations where
the public contract provides useful behavior but the official client's exact
sequence or lifecycle is unavailable. This section supersedes the earlier
"still unsupported" status as the current implementation status; the
Continuation B evidence remains historically accurate for the time it was
written. These implementations are **not** claims of exact Sonolus parity.

| Family | Implemented behavior | Boundaries and unresolved parity |
|---|---|---|
| Stack (14 functions) | Uses the end of the 4096-slot Temporary Memory block as a downward-growing stack. `StackEnter`/`StackLeave` save and restore stack/frame pointers; `StackPush`/`StackPop`, pointer getters/setters, frame access, and `StackGrow` operate on that region. Statically addressed ordinary Temporary Memory is reserved below the initial stack pointer. Bounds, underflow, invalid pointers, unbalanced leaves, and capacity exhaustion return errors. | Frame layout, pointer convention, callback/entity lifetime, unusual growth/leave sequences, and interaction with dynamically addressed Temporary Memory have not been compared with an official client. Dynamic access conservatively reserves the block and can leave no stack capacity. |
| `Random`, `RandomInteger` | Uses a reproducible, configurable SplitMix64 stream. `Random` returns a value in the documented closed range; `RandomInteger` uses the documented half-open integer range and rejection sampling. The seed is captured in `RenderConfig` and exposed by the CLI. | The official generator, seed source, exact sequence, degenerate-bound advancement, and lifecycle have not been established. The configured default is a renderer policy, not a Sonolus client default. |
| `StreamSet` | Returns `0`; updates an ordered `(stream id, key)` series, replacing an equal key. The store is shared with attached Replay Data for the lifetime of one Watch runtime. Entry count is bounded. Graphs containing stream operations run in ordered callback execution. | Duplicate-key policy, callback/frame visibility, replay mutation, and reset behavior may differ from the client. Stream state is runtime-local and is not persisted between renders. |

`RenderConfig.watch_compatibility_policy` defaults to `Permissive` for
backward compatibility. The `Strict` policy rejects any approximate family
before evaluating that operation's arguments, preventing its side effects.
Warnings are deduplicated by family through the shared job `ExportControl`;
render diagnostics retain them as observed events. The advanced render
settings expose the strict/permissive policy. `run-watch` and `render-video`
accept `--watch-compatibility-policy` and `--watch-random-seed`. Older
serialized render configurations omit these fields and deserialize to the
permissive policy and fixed renderer seed.

The analyzer reports these operation names as `Approximate`, counts them
separately from unsupported/unknown names, and keeps the Watch VM category
`Partial` when any are present. Sono-GCC does not compile these operations
directly: they are ordered WatchVm execution/cuts, and no native eligibility
was added. This retains the single WatchVm semantic authority and safe VM
fallback.

Focused tests cover stack operations and bounds, deterministic random ranges
and repeatability, stream insertion/replacement/query behavior, strict-mode
side-effect suppression, warning deduplication, serialized-config backward
compatibility, analyzer classification, and GCC exclusion. The real client
oracle is unavailable in this environment. Exact Sonolus compatibility for
these families remains **UNVERIFIED**. See [official Random](https://wiki.sonolus.com/engine-specs/functions/random),
[RandomInteger](https://wiki.sonolus.com/engine-specs/functions/random-integer),
[StreamSet](https://wiki.sonolus.com/engine-specs/functions/stream-set), and
[stack function overview](https://wiki.sonolus.com/engine-specs/functions/stack-functions)
for the public contracts used here.
