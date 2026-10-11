# Hosted administration console

The admin console is a local web client of the existing application services.
The Rust service owns the API contract and calls the render queue, history,
performance, diagnostics, cache, and profile owners. The React client displays
versioned DTOs; it does not calculate render timings, determine backend
fallback, or mutate queue state directly.

```text
Native GUI / CLI / browser
          |
          v
   Admin API (Axum)
      |       |       |
      v       v       v
 RenderJob  evidence  cache / profiles
 Manager    stores    owners
```

The service shares the same `Arc<RenderJobManager>` as the native GUI, so a
browser cancellation reaches the job manager's active cancellation control.
Queue actions use the existing manager methods. Performance comparisons use
`performance::compare_records`; cache operations use `cache_manager`; profile
operations use `ConfigProfileManager`. The service does not own or migrate
those stores.

## Build and serve

The frontend source is in `web/`. To change it, install the locked package set
and rebuild the embedded frontend:

```powershell
cd web
npm ci
npm run build
cd ..
cargo build --release
```

`web/dist` contains the small generated files embedded by the Rust binary and
is kept with the source release so ordinary Rust builds do not require Node.
`web/node_modules` is development-only and ignored. The server serves the
compiled page at `/`, the API under `/api/v1`, and requires no second process
for production use.

The React development server is available with `npm run dev`; it is bound to
loopback. A developer can run the API separately using the renderer CLI.
There is no permissive CORS configuration. The production page and API are
same-origin.

## Configuration and lifecycle

The versioned file `admin-server.json` lives under the application's existing
configuration root. If it is absent, the service is disabled and uses the
loopback address `127.0.0.1` and port `0` as safe defaults. Port zero asks the
operating system for a free port. A configured nonzero port is bound exactly;
an unavailable port is an error and is never silently replaced.

The native GUI exposes enable, bind, port, current listening URL, startup
errors, and an **Open Admin Console** link in **More → Admin Console**. Saving
these settings is atomic and requires an application restart. The CLI supports:

```text
renderer admin status [--json]
renderer admin configure --enable [--bind 127.0.0.1] [--port 0]
renderer admin validate
renderer admin run
```

`admin status` describes saved settings and does not claim to query a live
process. `admin run` starts the headless queue and console, prints the actual
bound URL, and exits cleanly on Ctrl+C.

## Network and authentication

Loopback is the only default. A loopback listener has no bearer token; the
security boundary is the local operating-system user and local browser. It is
not intended to distinguish multiple local users.

Non-loopback binding is supported only over HTTPS and requires:

- `SONO_RENDERER_ADMIN_TOKEN`, at least 32 bytes;
- `SONO_RENDERER_ADMIN_TLS_CERT`, a PEM certificate-chain path;
- `SONO_RENDERER_ADMIN_TLS_KEY`, a PEM private-key path.

These values are environment inputs, not fields in the settings file or API.
The token is compared by a constant-time byte comparison and is never returned
by the API. The browser keeps it in page memory only, sends it as a bearer
header, and does not persist it in local storage. Protected APIs require the
token; the unauthenticated health response is limited to status, version,
instance ID, and uptime. Responses use restrictive content-security and
no-store headers, and the service sets no cross-origin access policy.

For loopback listeners, the service checks the HTTP `Host` value against
loopback names/addresses and the actual port. This rejects arbitrary hostnames
that could otherwise be used in a browser DNS-rebinding attack. Remote HTTPS
listeners allow an operator-controlled hostname and rely on the required
bearer token and TLS configuration.

Remote access should be limited to a trusted network and firewall. For public
deployment, put the service behind an operator-managed TLS/authentication
boundary and do not expose it directly to the Internet. Certificate renewal,
multiple users, roles, token rotation, and enterprise identity are not part of
this version.

## API surface

All API routes are versioned under `/api/v1`.

| Route | Behavior |
|---|---|
| `GET /health` | Minimal liveness/version response |
| `GET /overview`, `GET /events` | Queue counts, recent failed history, latest performance, and 5-second SSE updates |
| `GET /jobs?limit=`, `GET /jobs/{id}` | Queue list capped at 500 (100 by default); list omits per-job logs/config, detail includes redacted logs and the effective configuration snapshot |
| `POST /jobs/{id}/cancel`, `POST /jobs/{id}/retry` | Existing queue cancellation/retry operations |
| `POST /queue/pause`, `POST /queue/resume` | Existing scheduler controls |
| `GET /history?text=&status=&limit=` | Bounded durable render-attempt search |
| `GET /performance`, `GET /performance/compare` | Authoritative records and comparability result |
| `GET /cache`, `GET /cache/{id}`, `POST /cache/validate` | Cache inventory, inspection, and explicit validation |
| `DELETE /cache/{id}`, `POST /cache/clear` | Destructive cache actions require explicit confirmation and use the cache manager's active-use protections |
| `GET /profiles`, `POST /profiles/select`, `PATCH/DELETE /profiles/{id}` | Profile listing, selection, rename, and confirmed deletion |
| `GET /diagnostics`, `GET /logs?text=&level=&limit=` | Structured diagnostics and filtered application/job log events; response reports process-capture availability |
| `GET/PUT /admin-config` | Safe admin service settings; changes take effect after restart |

History, job, performance, diagnostic, and log responses are bounded. The job
manager remains the authority for queue storage and lifecycle. Per-job log
messages, process log messages, and diagnostic summaries are redacted against
known secret environment variables. Process logs are held in a 2,000-entry
in-memory ring while the admin service is enabled; they are cleared on process
exit. Render-job events remain in the durable job/history stores. The logger
captures `log` facade events at Info, Warning, and Error levels; a host that
installed a different logger is reported as unavailable and its logs are not
captured. Job configuration is exposed only on the individual-detail route
and passes through the established history snapshot sanitizer. Profile APIs
return display summaries, not full configuration blobs. The cache clear
operation never includes durable source assets, projects, renders, profiles,
or history.

The SSE stream emits the same overview DTO every five seconds. A broken
connection is displayed as disconnected/stale in the web UI and reconnects
with a delay. The Logs page refreshes every five seconds only while it is open.

## Telemetry semantics and current limitations

Render performance and comparisons are read from `PerformanceStore` and use
the authoritative comparability logic, including effective VM/GCC fallback.
On Windows, the optional overview sampler reads process CPU time, system CPU
time, process working-set memory, and process read/write transfer counters
from native OS APIs. It samples only when an overview is requested, reuses a
snapshot for one second, and runs only while the admin service is enabled.
CPU and transfer rates are unavailable until two samples exist. Process
transfer counters do not represent physical disk throughput. GPU utilization,
VRAM, and physical disk throughput remain unavailable. Other platforms report
host metrics unavailable rather than inventing zero values.

The console provides queue inspection/control, historical attempts,
performance comparisons, cache inventory/validation/cleanup, profile
catalogue operations, diagnostics, and bounded process/job log search. It does
not currently provide notification administration, regression-laboratory
controls, arbitrary saved `RenderConfig` editing, output file download/reveal,
or domain-specific Sonolus views. Those systems remain owned by the native
GUI/CLI and were not duplicated in React. A future domain may add a narrow
typed view without making Sonolus a prerequisite of the admin service.

The service is started only when explicitly enabled. When disabled, no HTTP
listener, SSE stream, or admin sampling work runs. Sampling is synchronous
and bounded to at most once per second per instance; this is not a resource
history store or a performance benchmark. Admin idle/client overhead has not
been measured in the Performance Laboratory.
