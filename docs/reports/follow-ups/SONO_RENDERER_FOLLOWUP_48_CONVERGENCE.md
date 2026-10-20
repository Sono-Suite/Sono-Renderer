# Follow-up #48 — Historical Unfinished-Work Convergence

**Review date:** 2026-10-08  
**Repository identity:** `fe8dc1e` plus a pre-existing dirty working tree. This pass did not create a clean/tagged release identity.

## Decision summary

This pass reconciled prior reports with current source and closed one feasible hosted-admin implementation gap: on Windows, the optional admin overview now reports OS-derived process/system CPU, process working-set memory, and process I/O rates. GPU, VRAM, and physical disk throughput remain unavailable.

The historical ledger is **not fully green**. Incomplete Sonolus v1.1.4 operations, incomplete application localization, physical/model/network/GUI validation gaps, and external release/legal prerequisites remain. This report does not relabel them as complete.

## Evidence taxonomy

- **VERIFIED** — current production path has direct automated, integration, or real-media evidence, within the stated fixture/platform scope.
- **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** — implementation exists, but required GUI, device, model, hardware, clean-machine, or external-service evidence was unavailable.
- **BLOCKED_EXTERNAL** — completion depends on an external input, rights-holder, release tool, device, model, service, or maintainer decision.
- **BLOCKED_CORRECTNESS** — a material implementation or semantic gap remains.
- **SUPERSEDED** — a later architecture/workflow replaced the historical form.
- **INTENTIONALLY_REJECTED** — a later architecture decision explicitly rejects the proposed behavior or parallel system.
- **OUT_OF_SCOPE** — not an accepted current product claim.

Source presence, a GUI control, a mock, or a successful build alone is not workflow proof.

## Baseline

Before the production change, `cargo fmt --all -- --check` passed and `cargo test --locked --all-targets` passed on Windows x86_64: library 432 passed / 8 ignored, CLI 27, `arctan2` 3, `m0` 101, Watch scheduler 9 (572 passed, 8 ignored). No process abort occurred. This does **not** close the historical Windows parallel-test abort; no root cause was established.

The working tree was already dirty with broad pre-existing changes and artifacts. No commit, tag, cleanup, or release-source claim was made. See [#39 certification](../audits/RELEASE_CANDIDATE_CERTIFICATION.md) and [#40.5 stabilization](SONO_RENDERER_FOLLOWUP_40_5_STABILIZATION.md) for release identity and package evidence.

## Production change: hosted admin host-resource sampling

**Previous state:** #47 classified host telemetry as partial/unavailable. The overview returned null for every resource metric.

**Implemented:** `src/admin.rs` samples Windows process and system counters on overview requests. The API now exposes process CPU, system CPU, process working set, and process read/write transfer rate. CPU values use OS counter deltas normalized by available logical processors. The frontend presents those separately.

**Explicit limits:** process transfer counters are not physical disk throughput. GPU utilization, VRAM, and physical disk throughput remain null/unavailable. Non-Windows builds report host metrics unavailable. No background sampler or sample history exists. Sampling is request-driven, rate-limited to one sample per second, and only reachable when the optional admin service is running. The existing five-second SSE cadence is unchanged. Idle/client overhead was not benchmarked in the Performance Lab.

**Safety:** the Windows API calls use the OS-owned current-process pseudo-handle and stack-owned ABI structures. No persistent handle or user data is added.

## Consolidated historical feature matrix

| Area | Prior unresolved state | #48 status | Evidence / boundary |
|---|---|---|---|
| Direct Sonolus render | Narrow real fixture path passed | **VERIFIED** | #39/#36 real Larp engine + level + resources/music → H.264/AAC. Fixture-scoped, not universal engine support. No Presentation Studio dependency. |
| One Watch traversal | Risk of parallel gameplay runtimes | **VERIFIED** | One authoritative `WatchVm`/Watch lifecycle path remains. Sono-GCC uses VM cuts/fallback; no project/history/admin/NLE Watch evaluator was added. |
| Generic Sonolus v1.1.4 completeness | Stack APIs, replay host provisioning, `StreamSet` edge behavior, curved Draw rasterization, and some rendering/particle semantics remain incomplete or unresolved | **BLOCKED_CORRECTNESS** | See `docs/SONOLUS_V1_1_4_CAPABILITY_AUDIT.md` and `docs/SONOLUS_V1_1_4_ORACLE_ARITY_AUDIT.md`. Pointer/frame layout, stream lifetime, or curved pixels were not guessed. |
| Watch/media time | Distinct clocks and offsets | **VERIFIED** | Existing tests preserve Watch/media mapping, frame/sample conversion, and composition/source time boundaries. This pass did not change time code. |
| Sono-GCC | Parity shown on one fixture; no general speedup | **VERIFIED** on the checked fixture; broader claims **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | #39 Larp output was hash-identical to VM; cold compile was much slower and warm run not faster. VM remains oracle/default; GCC remains optional. See `SONO_GCC_OPTIMIZATION_VERIFICATION.md`. |
| Chart Studio canonical editing/export | Generic structural authoring; no universal engine semantics | **VERIFIED** for model/export; interactive workflow **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Tests cover unknown-field preservation, atomic gzip export/reimport, constraints, undo/redo, and selected engines. It does not infer arbitrary lane/note/relationship semantics. |
| Engine-authored authoring schema | No universal declaration is supplied by arbitrary engines | **BLOCKED_EXTERNAL** | Optional identity-bound schema support exists. A content owner must supply schema/metadata; automatic semantic inference is not safe. |
| Standalone Presentation Studio | Ordinary media production pipeline passed | **VERIFIED** for production pipeline; mouse UX **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Composition does not require EngineData, LevelData, WatchVm, or Sono-GCC. FFmpeg media/audio/text/shape/effect/save/export tests exist. A human desktop editing session was unavailable. |
| Baseline NLE editing | Timeline, import/relink, proxy, playback, audio, transitions, persistence exist | **VERIFIED** at model/FFmpeg boundaries; interaction **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | #36 audit covers edit operations and production filter graphs. Mouse gestures, physical playback, sustained drift, and dropped frames were not manually certified. |
| Advanced #37 customization | Freeform masks, rich text spans, expanded blends/transitions, full effect animation and pro controls remain partial/absent | **BLOCKED_CORRECTNESS** | #40.5 deferred expansion during stabilization, but did not permanently reject the explicit #37 requests. They remain genuine implementation gaps; this pass did not complete them or claim a professional NLE. |
| Live Watch-backed generic NLE source | Composition accepts rendered media, not a live Watch document | **INTENTIONALLY_REJECTED** | #40.5/#46 preserve the generic composition boundary. Gameplay can be rendered in its domain and used as ordinary media. |
| Reusable/responsive templates | Valid landscape/portrait fixtures; simple normalized/full-canvas rules | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** for visual quality | Prior files were FFprobe-validated and inspected. Portrait density and multi-view usability need human review; no general breakpoint/constraint solver exists. |
| Shorts acceptance | Portrait export fixture passed; full multi-view gameplay/caption/voiceover flow not run | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | 1080×1920 output exists; exact interactive workflow was not certified. Spatial scale is separate from playback rate. |
| Audio mix/edit | Offline audio pipeline works; real output-device/long-sync behavior unavailable | **VERIFIED** for offline production paths; physical playback **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Tests cover gains, fades/crossfades, mute/solo/pan, source audio, and output streams. No DAW or alternate clock was added. |
| Microphone | Capture/finalization/recovery tests exist; no physical device | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Permissions, hot-plug, long drift, and packaged capture were unavailable. No audio engine rewrite was warranted. |
| Automatic subtitles | Timed text, SRT/VTT, Unicode editing and audio extraction exist; no model installed | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No real-speech accuracy, multilingual quality, or packaged inference claim. Accepted cues remain project data without the model. |
| BPM / phase | Synthetic audio, silence/noise, fractional tempo and phase tests pass | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No licensed/labeled real-song corpus or human ground truth was available. No arbitrary-song quality claim. |
| URL acquisition | GUI/CLI and safe process boundary exist; yt-dlp/network unavailable | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No live URL was acquired. Local file import is independent of yt-dlp. |
| Project/profile/job/history persistence | Versioned stores and immutable job snapshots exist | **VERIFIED** for automated model/store/queue/render paths | Serialization, migration, path safety, queue render, cancellation/retry, and history tests remain authoritative. GUI repair/recovery was not manually exercised. |
| Crash recovery | Atomic recovery checkpoints and dirty-close decision exist; bounded loss window remains | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Model/preservation tests pass. No GUI kill/relaunch or disk-fault injection; sudden power loss can lose work since last checkpoint. |
| Performance/diagnostics/cache/regression/manifests | Authoritative stores exist; real cross-version compare/replay not completed | **VERIFIED** for store/model paths; exact replay **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No duplicate timer, history, diagnostic, cache identity, or manifest model was added. |
| Notifications | Mock/retry/redaction tests; no real OS/Discord delivery | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No provider delivery attempted; notification failure remains independent from rendering. |
| Application localization | Locale/catalog infrastructure exists, but most UI literals remain English | **BLOCKED_CORRECTNESS** | `../audits/INTERNATIONALIZATION_LOCALIZATION_REPORT.md` says #43 remains partial. Missing extraction/translation is an implementation gap; catalogs do not prove application coverage. |
| Onboarding/workspace/commands | State and route logic tested; installed interactive first run unavailable | **VERIFIED** for state logic; GUI **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | No clean installed GUI session. |
| Hosted admin API/frontend | Versioned API, authoritative services, security checks, SSE and embedded React | **VERIFIED** for local routes/build; remote TLS/browser **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Existing #47 API tests plus this pass. No public/LAN deployment certification. |
| Hosted admin host telemetry | Previously returned null for all metrics | **VERIFIED** for Windows native counter path and DTO math | This pass adds process/system CPU, process working set, and process I/O. GPU/VRAM/physical disk remain honestly unavailable. Admin overhead not benchmarked. |
| Process-wide logs | No authoritative process-wide logging owner; current page shows bounded job events only | **BLOCKED_CORRECTNESS** | #47 explicitly requested live/historical logs. The current job-event view does not fulfill that broader requirement; a safe authoritative logging owner is not implemented. |
| Arbitrary config editor, fleet control | No user need/authority model for unrestricted config editing or multiple hosts | **OUT_OF_SCOPE** | No speculative configuration service locator or fleet manager. |
| Release platform | Windows x86_64 declared target | **VERIFIED** for current Windows build/test scope; other OS packages **OUT_OF_SCOPE** | No Linux/macOS release claim. |
| Installer/version resource/clean machine | `ISCC.exe`, `rc.exe`, clean VM unavailable | **BLOCKED_EXTERNAL** | Installer lifecycle, PE resource, clean installed GUI remain external release-environment work. |
| Legal redistribution | Asset provenance and missing license texts require review | **BLOCKED_EXTERNAL** | Rights/legal conclusions require maintainer/rights-holder review. |
| FFmpeg clean/offline packaging | Shared development FFmpeg worked; bootstrap/redistribution unverified | **BLOCKED_EXTERNAL** | Clean offline install and distribution policy were not certified. |
| Historical Windows parallel abort | Prior unexplained harness instability; current run clean | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | One passing run is not a root-cause fix; keep incident open. |
| DAW/MIDI/VST, arbitrary plugin marketplace, cloud/fleet | Not current product capability | **INTENTIONALLY_REJECTED** / **OUT_OF_SCOPE** | #37 rejects DAW scope; #46 rejects speculative plugin/job/document abstractions. |

## Gap register

| ID | Gap | Classification | Result |
|---|---|---|---|
| H01 | Admin had no process/system sampler | Implementation/integration | **Closed for Windows process/system CPU, working set, process I/O.** |
| H02 | Process transfer could be mislabeled as physical disk throughput | Correctness/presentation | **Closed:** distinct process I/O field; physical disk remains null. |
| H03 | Frequent dashboard requests might create sampling load | Performance/reliability | **Closed by bounded design:** no worker, one-second sample reuse, disabled service has no sampler path. No overhead benchmark claim. |
| H04 | #43 application-wide localization incomplete | Implementation | **Open: BLOCKED_CORRECTNESS.** No bulk machine translation or weakened criterion. |
| H05 | Watch stack/replay/curved draw semantics incomplete | Correctness/semantics | **Open: BLOCKED_CORRECTNESS** pending exact spec/oracle evidence. |
| H06 | Microphone/STT/URL/BPM/GPU evidence unavailable | External/environment | **Open: BLOCKED_EXTERNAL** for missing device, model, network/tool, licensed corpus, or adapter. |
| H07 | Clean package/installer/release rights | Packaging/external | **Open: BLOCKED_EXTERNAL** pending Windows tooling, clean host, release identity, and legal review. |
| H08 | Long playback, GUI crash recovery, exact feature-stress workflows | Verification | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** where code exists; interactive/fault-injection evidence absent. |
| H09 | Explicit #37 style/customization controls remain partial/absent | Implementation | **Open: BLOCKED_CORRECTNESS.** #40.5 deferred it for stabilization; that was not a permanent rejection of #37's accepted requests. |
| H10 | Live Watch runtime in generic composition | Domain boundary | Not reopened; gameplay remains a domain render that may enter as ordinary media. |
| H11 | #47 process-wide live/historical logs | Implementation | **Open: BLOCKED_CORRECTNESS.** The existing Logs page is bounded job-event history, not an application log stream. |

## Validation after the change

| Validation | Result |
|---|---|
| `cargo fmt --all -- --check` | Passed. |
| `cargo check --all-targets` | Passed; existing Windows build warning reports `rc.exe` unavailable for version metadata. |
| `cargo test --locked --all-targets` | Post-change run: library 435 passed / 8 ignored, CLI 27, `arctan2` 3, `m0` 101, Watch scheduler 9 (575 passed, 8 ignored). No failure or process abort. |
| `cargo clippy --locked --all-targets` | Passed with the repository's existing warnings; no warning was emitted for the new sampler arithmetic. |
| `cargo build --locked --release` | Passed. Existing warning: `rc.exe` unavailable, so Windows version metadata was not embedded. |
| Resource arithmetic | Unit-tested processor normalization, clamping, system idle/kernel/user deltas, and empty intervals. |
| Windows native sampler | Windows-only test reads current-process working set. |
| Hosted API DTO | Local server integration checks nullable CPU/I/O fields on the first sample and API serialization. |
| React/TypeScript production build | TypeScript and Vite passed using installed Node 26 directly. The `npm` nvm shim had no active version; the underlying frontend build succeeded. |
| Release-binary admin smoke | Passed with isolated `LOCALAPPDATA`: `admin run` bound to `127.0.0.1:54449`; two `/api/v1/overview` requests reported process CPU 0.0% then 0.063%, system CPU 16.94%, process working set 13,115,392 bytes, process I/O 0.0 B/s, and physical disk I/O `null`. `/admin.js` served the new label. The process stopped on Ctrl+C. These are one host's smoke values, not a benchmark. |
| Interactive browser / remote TLS | Not tested. |
| Microphone, model inference, yt-dlp network, labeled BPM corpus, physical GPU | Unavailable; no device/service quality claim. |
| Installer, clean Windows VM, signing, Linux/macOS package | Unavailable / unsupported release target. |
| Performance Lab overhead | Not measured; no second timer introduced. |
| Real-engine render after this change | Not rerun because only admin telemetry/frontend changed. Existing #39 real Larp evidence and post-change full semantic tests are cited; no renderer/time code changed. |

## Remaining blocker matrices

### BLOCKED_EXTERNAL

- Dirty/untagged release source; maintainer review, commit, matching release notes, and tag are required.
- `rc.exe` and Inno Setup `ISCC.exe` are unavailable; installer/version-resource and install/upgrade/uninstall tests remain open.
- Asset/font/media redistribution rights and missing dependency license texts require human review.
- Clean-machine GUI and offline FFmpeg bootstrap/redistribution were not tested.
- Physical microphone/permissions, approved speech model and real speech, yt-dlp/network, licensed BPM corpus, physical GPU, and remote TLS/browser session were unavailable.

### BLOCKED_CORRECTNESS

- Generic v1.1.4 Watch compatibility remains incomplete: stack APIs, replay host provisioning, stream-write edge semantics, curved Draw, and other audit-specific gaps.
- Complete application localization remains false: most UI and error messages are not catalog-backed.
- Explicit #37 presentation controls remain incomplete, including freeform masks, rich text spans, some effects/transitions/blends and parameter animation.
- The #47 Logs page remains job-event scoped; no process-wide structured logging history exists.
- Any release claim depending on a partial area above must stay limited until the exact missing path/evidence is closed.

### INTENTIONALLY_REJECTED / OUT_OF_SCOPE

- No second Watch runtime, universal inferred chart semantics, live Watch runtime embedded in generic composition, DAW/MIDI/VST, speculative plugin host, universal job/document/asset framework, or multi-host fleet was added.
- Linux/macOS release packages are outside the declared Windows x86_64 support matrix.

### SUPERSEDED

- A separate standalone video-editor application is superseded by one Presentation Studio that accepts ordinary media without Sonolus.
- Routing every render through the NLE is superseded by preserving direct engine + level + resources/music → MP4.

## Acceptance-project evidence

| Acceptance | Status | Evidence / blocker |
|---|---|---|
| Engine + level + resources/music → MP4 | **VERIFIED** | Prior real Larp H.264/AAC export and matching hash; post-change full suite passed. |
| Ordinary media → Presentation Studio → edit/export, zero Sonolus | **VERIFIED** for production pipeline | Existing ordinary MP4/WAV/still/title/effect and composition persistence fixtures. Mouse-driven GUI remains untested. |
| 1080×1920 composition | **VERIFIED** for portrait fixture; full custom Shorts flow **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Prior portrait output was FFprobe-validated; full multi-view gameplay/caption/voiceover GUI workflow not run. |
| Chart author → gzip LevelData → reopen → Watch → MP4 | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Model/export/reimport/Watch fixtures pass; full GUI chart-edit-to-MP4 round trip absent. |
| Gameplay + recorded voiceover + transcription + styled captions | **BLOCKED_EXTERNAL** | Requires interactive GUI, physical microphone, and local STT model; current gameplay source is rendered media. |
| Save/reopen/recover and continue editing | **VERIFIED** for store/model tests; GUI crash/relaunch **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Atomic recovery tests exist; no interactive kill/relaunch harness. |
| Hosted admin | **VERIFIED** for API, frontend build, Windows resource-counter path, and release-binary local HTTP smoke | Remote TLS, installer, and clean machine not tested. |
| Historical manifest rerender | **IMPLEMENTED_BUT_ENVIRONMENTALLY_UNVERIFIED** | Reconstruction tests exist; no real historical attempt re-rendered in this pass. |

## Module-by-module closure summary

| Module | Decision |
|---|---|
| Repository archaeology | Current reports and source were used; old prompt text was not treated as proof. |
| Shared/domain architecture | #46 boundaries remain; composition/admin do not require Watch and Sonolus stays domain-owned. |
| Persistence/data safety | No schema changed; existing owners and migrations remain authoritative. |
| Jobs/history/telemetry | Existing stores remain the source of truth; no duplicate timing, history, diagnostic, or job model. |
| Media/NLE/audio/subtitles | Existing boundaries remain; missing physical/model inputs did not trigger a rewrite. |
| Web admin | Windows host sampler implemented. GPU, physical disk, process-wide logs, general RenderConfig editor, and fleet control remain absent. |
| CLI/native GUI | No CLI or desktop workflow contract changed. Embedded React source/build updated. |
| Packaging/release | No installer/clean-machine claim; blockers documented. |
| Test architecture | Existing suites were rerun; sampler tests protect arithmetic, caching, API DTO shape, and Windows memory counter access. |

## Conclusion

One concrete accepted implementation gap is closed. The rest of the ledger is
separated into verified behavior, named correctness gaps, and blocked evidence
or release inputs. Sono-Renderer is not certified as fully compatible with all
Sonolus engines, fully localized, a professional NLE, or a clean-install
release artifact. Product claims should stay within the verified boundaries.

**Overall convergence: NOT COMPLETE.** The remaining `BLOCKED_CORRECTNESS` rows
are real accepted implementation gaps, not environmental excuses. The #39
release recommendation remains **NOT READY FOR RELEASE CANDIDATE** until the
release-source/legal/installer/clean-machine gates are closed by their owners.
