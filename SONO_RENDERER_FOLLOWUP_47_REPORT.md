# Follow-up #47 — Hosted Web Admin / Operations Console

## Result

**IMPLEMENTED:** an optional, same-process Axum administration API with an
embedded React/TypeScript web client, CLI lifecycle/configuration commands,
and native-GUI settings/status access. The web client calls the existing
queue, history, performance, diagnostics, cache, and profile owners. It does
not calculate renderer results or own those systems' persistence.

The repository was already heavily modified and contained numerous
uncommitted and untracked files before this task. This report describes the
#47 changes and validation; it does not claim a clean Git baseline or assign
the unrelated existing workspace changes to #47.

## Architecture found

The #46 architecture report and current source show one Rust package/crate.
`main.rs` owns the Clap command tree, `gui.rs` owns the eframe application
shell, and subsystem modules own their state and persistence. There was no
HTTP server or web frontend. Authoritative services already existed for
render jobs, render history, performance, diagnostics, cache administration,
and configuration profiles. The render queue remains render-specific; this
work does not replace it with a generic executor.

The new dependency direction is:

```text
Native GUI / CLI / browser
            |
            v
       admin API (DTOs)
        /    |     \
       v     v      v
   job owner evidence stores cache/profile owners
```

`admin.rs` is an adapter/control surface. The browser is not a dependency of
rendering, jobs, or the native GUI. When started by the GUI, it receives the
same `Arc<RenderJobManager>` instance so cancellation and queue controls reach
the authoritative manager. The separate `admin run` CLI starts a headless
instance using the normal persistent render queue.

## Modules 0–24

| Module | Status | Finding / implementation |
|---|---|---|
| 0. Architecture inspection | **IMPLEMENTED** | Inspected the current Rust owners, CLI/GUI, #46 boundaries, and actual stores. See [Admin Console docs](docs/ADMIN_CONSOLE.md) and [General Platform Architecture](docs/GENERAL_PLATFORM_ARCHITECTURE.md). |
| 1. Admin server | **IMPLEMENTED** | Optional Axum service on a dedicated thread/runtime; starts only when explicitly enabled. Bound address is reported. |
| 2. Network binding | **IMPLEMENTED** | Versioned config supports literal IPv4/IPv6 and a configurable `u16` port. Default is `127.0.0.1:0`; port zero asks the OS for an ephemeral port. A requested nonzero port is used exactly; bind failure is an error. Shutdown is graceful. |
| 3. Authentication | **IMPLEMENTED WITH LIMITS** | Loopback is unauthenticated and relies on the local OS user boundary. Loopback requests validate Host against loopback names/addresses and the actual port to reject DNS-rebinding hostnames. Non-loopback binding requires a 32-byte minimum bearer token and PEM TLS certificate/key from environment variables. No secret is persisted or returned. There are no user/role accounts or token rotation UI. |
| 4. API | **IMPLEMENTED** | Versioned `/api/v1` endpoints expose stable DTOs. API payloads are not persisted internal structs by default; job detail uses the existing secret-sanitized config snapshot. Overview updates use Server-Sent Events at five-second intervals. |
| 5. Frontend | **IMPLEMENTED / BUILD-CHECKED** | Responsive React 19 + TypeScript + Vite client with Overview, Jobs, History, Performance, Caches, Profiles, Diagnostics, Logs, and Admin Settings views. It indicates stale/disconnected state and reconnects. Operator token remains in page memory. |
| 6. Overview | **IMPLEMENTED** | Shows version/uptime/instance health, queue counts, recent failures, and latest performance. Unavailable host metrics are JSON `null`, explicitly labeled unavailable; they are not displayed as zero. |
| 7. Host telemetry | **PARTIAL / NOT AVAILABLE** | Process/system CPU, memory, GPU/VRAM, disk I/O, and network metrics are not supplied by an authoritative sampler in this repository. No synthetic metrics were added. |
| 8. Performance Lab | **IMPLEMENTED** | Reads `PerformanceStore`, stage timings, environment/workload metadata, cache/backend status, and calls the existing `compare_records` comparability logic. The console does not relabel fallback VM execution as GCC. |
| 9. Job management | **IMPLEMENTED** | Lists jobs (100 default, maximum 500), omits logs/config from list payloads, shows captured config and bounded logs on detail, and calls manager cancellation/retry/pause/resume operations. Job listing prioritizes active work, then most recently finished work. |
| 10. Cache management | **IMPLEMENTED** | Uses `cache_manager` inventory/inspect/validate/remove/clear. Destructive requests require explicit confirmation and still pass through existing in-use protections. Opening inventory is read-only. |
| 11. Configuration/profiles | **PARTIAL** | Lists/selects/renames/deletes profiles through `ConfigProfileManager`; native GUI synchronizes the selected profile. Admin settings support enable/bind/port. A general arbitrary `RenderConfig` editor is intentionally not exposed. Jobs retain their captured snapshots. |
| 12. Diagnostics | **IMPLEMENTED FOR RENDER EVIDENCE** | Reads structured reports linked from render history and redacts summaries. No terminal scraping. It does not present a generalized platform-health/plugin registry because none exists. |
| 13. Logs | **PARTIAL** | Searchable bounded job-manager logs, capped at 500 returned entries and drawn from active plus recent jobs. This is not a process-wide live log aggregator; no such authoritative facility exists. |
| 14. History/regression | **PARTIAL** | Bounded render-history search and performance comparison are integrated. Regression Lab controls and arbitrary artifact export are not included. |
| 15. Domain views | **FUTURE** | Current console is platform-oriented and does not require Sonolus. There is no speculative dynamic view registry and no Sonolus-specific admin panel yet. |
| 16. Command/control safety | **IMPLEMENTED** | No shell-command endpoint. Destructive cache/profile operations require explicit confirmation. Queue actions invoke existing manager APIs. TLS/auth are mandatory for non-loopback binding. |
| 17. Multi-host seam | **FUTURE** | A stable per-instance ID is exposed by health. Fleet management is not implemented. |
| 18. Build/packaging | **IMPLEMENTED / RELEASE-BINARY TESTED** | React assets are embedded in the Rust executable from `web/dist`; production does not need a second process, Node, source checkout, or frontend server. `web/node_modules` is ignored. Installer artifacts were not exercised in this task. |
| 19. CLI | **IMPLEMENTED** | Adds `renderer admin status [--json]`, `validate`, `configure`, and `run` under the existing CLI. `status` reports saved config, not a live process. `run` prints its actual URL and exits on Ctrl+C. |
| 20. Native GUI | **IMPLEMENTED / BUILD-CHECKED** | The More menu shows service status/error and current URL, provides an open-console link, and allows saving enable/bind/port settings. Changes require restart. The React UI is opened in the external browser; no embedded browser was added. |
| 21. Tests | **IMPLEMENTED / PARTIAL COVERAGE** | Backend tests cover config defaults/validation/atomic save, bind conflicts, token comparison and prerequisites, route authorization/security headers, loopback Host validation including a rebinding hostname, embedded frontend, and API list/SSE route responses. There is no separate React component-test framework. |
| 22. Overhead | **NOT MEASURED** | No Performance Lab record or connected-client benchmark exists for admin overhead. With the server disabled, there is no listener, SSE stream, or admin sampler. |
| 23. Documentation | **IMPLEMENTED** | Added architecture, config, build, API, security/deployment, telemetry-semantics, CLI, and limitations documentation; linked it from README and the #46 platform boundary document. |
| 24. Final validation | **SEE EVIDENCE BELOW** | Backend, frontend, CLI, release executable, and repository tests were validated to the levels reported below. No remote deployment, installer, or interactive GUI/browser session was available. |

## API and frontend

The main endpoints are:

- `GET /api/v1/health`, `GET /api/v1/overview`, `GET /api/v1/events`
- `GET /api/v1/jobs?limit=`, `GET /api/v1/jobs/{id}`, job cancel/retry, queue pause/resume
- `GET /api/v1/history`, `GET /api/v1/performance`, `GET /api/v1/performance/compare`
- cache inventory/inspection/validation/removal/clear
- profile list/select/rename/delete
- structured diagnostics and bounded job logs
- `GET/PUT /api/v1/admin-config`

The overview SSE stream is bounded to one update every five seconds per
connected client. There is no aggressive frontend polling loop. The UI
reconnects after transient disconnects and labels cached overview information
stale until it receives a fresh update.

## Configuration and security

`AdminServerConfig` is a version-1, unknown-field-rejecting JSON document at
the existing application config root (`admin-server.json`). It is disabled
by default, saved atomically, and separate from render profiles because
network administration settings are not render preferences. The default
loopback port `0` is ephemeral; set a concrete port for a stable local URL.

For a non-loopback bind, configure all of:

- `SONO_RENDERER_ADMIN_TOKEN` (at least 32 bytes)
- `SONO_RENDERER_ADMIN_TLS_CERT` (PEM certificate chain)
- `SONO_RENDERER_ADMIN_TLS_KEY` (PEM private key)

The token remains in the process environment and browser memory only. Remote
TLS configuration is loaded by Rustls; a malformed certificate/key makes
startup fail. Health and the static client are public; operational APIs
require the bearer token. The local unauthenticated model is only for one
trusted OS user on that machine. Host validation mitigates browser DNS
rebinding, but it is not a substitute for remote authentication. Public
Internet exposure, multiple users/roles, token rotation, and certificate
renewal automation are not certified.

## Validation evidence

### Build and tests

- `cargo fmt --all -- --check`: **passed** after formatting the final Rust changes.
- `cargo check --all-targets`: **passed**; Windows build metadata warns that `rc.exe` is unavailable, so the executable version resource was not embedded.
- `cargo test --lib admin::tests -- --nocapture`: **11 passed** on the final source, covering Host validation, queue pause/resume, API list routes, and short-secret redaction.
- `cargo test --all-targets`: **passed** — library 431 passed / 8 ignored; CLI 27 passed; oracle 3 passed; `m0` integration 101 passed; Watch scheduler 9 passed; examples 0 tests. No failures. This all-target run preceded the last two small admin-only changes: omitting logs from list DTOs and redacting short nonempty secret values.
- TypeScript `tsc --noEmit` and Vite production build: **passed**. The normal Vite config bundler hit this restricted shell's inaccessible parent path; the same production build passed using Vite's supported `--configLoader runner` mode.
- `cargo run --quiet -- admin validate`: **passed**, reporting the default configuration disabled at `127.0.0.1:0`.
- `cargo build --release`: **passed** after the final production source change. Windows build metadata warns that `rc.exe` is unavailable, so executable version metadata was not embedded.
- Release-binary smoke test using isolated temporary `LOCALAPPDATA`: CLI configuration succeeded; `admin run` bound to `127.0.0.1:60270`; health, overview, `/`, embedded JS, and embedded CSS returned **200**; an attacker-controlled Host value returned **400**; host metrics were `null`/unavailable. The process was stopped and the temporary data directory removed.

The focused admin suite was rerun after the final changes:
**11 passed, 0 failed**. It includes queue pause/resume, protected list routes,
Host validation, and short-value redaction. `cargo fmt --all -- --check`
passed after the final Rust edits. `cargo check --all-targets` passed before
the final admin-only changes; the focused tests and release build compiled
the exact final source.

### Not exercised

- No actual remote TLS session or browser login was performed.
- No interactive native GUI or external-browser workflow was available to exercise; native code is compile-checked.
- No installer/package archive was tested, only the release executable with embedded web resources.
- No physical GPU/host sampler metrics exist to validate, and admin idle/client overhead was not measured.
- No live render was started through the web console. Job actions are tested/integrated against the existing manager API; the test suite's renderer fixtures remain the render authority.
- No clean second machine or public/LAN deployment was tested.

## Intentional limits and future work

This task does not create a process-wide logging system, host resource
sampler, notification control panel, regression-lab controls, arbitrary
configuration editor, output download/reveal API, Sonolus-specific panel,
multi-host fleet, account/role system, or enterprise identity. Those are
**FUTURE**, not silently represented as implemented. Host telemetry and a
global live log view are the clearest remaining gaps for the requested broad
operations-console scope. A general profile editor and authenticated remote
browser session are also not certified.

The service uses one bearer-token admin role remotely, so any valid token has
full operational access. Protect environment values and TLS private keys
with OS-level permissions, firewall remote access, and do not expose an
untested deployment directly to a public network.
