import React, { useCallback, useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import "./style.css";

type Page = "overview" | "jobs" | "history" | "performance" | "cache" | "profiles" | "diagnostics" | "logs" | "settings";
type Health = { status: string; version: string; uptime_seconds: number; instance_id: string };
type Overview = {
  jobs: { queued: number; active: number; completed: number; failed: number; cancelled: number; paused: boolean };
  recent_failures: History[];
  latest_performance?: Performance;
  resources: { process_cpu_percent: number | null; system_cpu_percent: number | null; process_memory_bytes: number | null; process_io_bytes_per_second: number | null; gpu_utilization_percent: number | null; vram_bytes: number | null; disk_io_bytes_per_second: number | null; availability: string };
};
type Job = {
  id: string; state: string; priority: string; attempt: number; submitted_unix_time_ms: number;
  started_unix_time_ms?: number; finished_unix_time_ms?: number; profile_name?: string;
  renderer_version: string; execution_backend: string; phase?: string; completed_frames: number;
  total_frames?: number; percentage?: number; output_name?: string; output_exists: boolean;
  cancellation_requested: boolean; diagnostic?: string; failure_kind?: string;
  logs: { unix_time_ms: number; message: string }[];
  configuration_snapshot?: Record<string, unknown>;
};
type History = { id: string; job_id: string; status: string; renderer_version: string; execution_backend: string; profile_name?: string; completed_unix_time_ms: number; performance_record_id?: string; diagnostic_record_id?: string; output_name?: string; error?: string };
type Stage = { name: string; duration_ms: number; samples: number; semantics: string };
type Performance = { id: string; execution_time_ms: number; queue_wait_time_ms?: number; total_user_latency_ms?: number; effective_backend_status: string; execution_mode: string; backend: string; renderer_version: string; resolution: [number, number]; fps: number; frames: number; cache_state?: string; stages: Stage[]; environment: { operating_system: string; architecture: string; cpu_identity?: string; gpu_identity?: string } };
type Cache = { total_disk_bytes: number; regenerable_disk: { category: string; entries: number; bytes: number }[]; disk_entries: { id: string; category: string; size_bytes: number; status: string; schema?: number; target?: string }[]; memory: { category: string; entries: number; retained_bytes?: number; limit_bytes?: number; notes: string }[] };
type Profile = { id: string; name: string; active: boolean; updated_unix_time_ms: number; resolution: [number, number]; fps: number; execution_mode: string };
type Diagnostic = { id: string; generated_unix_time_ms: number; outcome: string; effective_backend: string; findings: { kind: string; code: string; summary: string }[]; performance_record_id?: string };
type AppConfig = { schema_version: number; enabled: boolean; bind_address: string; port: number; authentication: string; restart_required: boolean };
type LogEntry = { source: "application" | "job"; job_id: string | null; state: string | null; level: string | null; target: string | null; unix_time_ms: number; message: string };
type LogsResponse = { application_capture_active: boolean; application_retention_limit: number; entries: LogEntry[] };

const nav: { id: Page; label: string; icon: string }[] = [
  { id: "overview", label: "Overview", icon: "◫" }, { id: "jobs", label: "Jobs", icon: "▤" },
  { id: "history", label: "History", icon: "◷" }, { id: "performance", label: "Performance", icon: "⌁" },
  { id: "cache", label: "Caches", icon: "▦" }, { id: "profiles", label: "Profiles", icon: "◉" },
  { id: "diagnostics", label: "Diagnostics", icon: "⌕" }, { id: "logs", label: "Logs", icon: "≡" },
  { id: "settings", label: "Admin settings", icon: "⚙" },
];

async function request<T>(path: string, token: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  if (token) headers.set("Authorization", `Bearer ${token}`);
  if (init?.body) headers.set("Content-Type", "application/json");
  const response = await fetch(path, { ...init, headers, cache: "no-store" });
  if (response.status === 401) throw new Error("AUTH_REQUIRED");
  const payload = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(payload.error || `Request failed (${response.status})`);
  return payload as T;
}

function App() {
  const [page, setPage] = useState<Page>("overview");
  const [token, setToken] = useState("");
  const [health, setHealth] = useState<Health | null>(null);
  const [overview, setOverview] = useState<Overview | null>(null);
  const [connected, setConnected] = useState(false);
  const [authRequired, setAuthRequired] = useState(false);
  const [error, setError] = useState("");
  const [revision, setRevision] = useState(0);
  const [selectedJob, setSelectedJob] = useState<string | null>(null);

  const loadOverview = useCallback(async () => {
    try {
      const [h, o] = await Promise.all([
        request<Health>("/api/v1/health", ""),
        request<Overview>("/api/v1/overview", token),
      ]);
      setHealth(h); setOverview(o); setConnected(true); setAuthRequired(false); setError("");
    } catch (cause) {
      const message = cause instanceof Error ? cause.message : String(cause);
      if (message === "AUTH_REQUIRED") { setAuthRequired(true); setConnected(false); }
      else { setConnected(false); setError(message); }
    }
  }, [token]);

  useEffect(() => { void loadOverview(); }, [loadOverview, revision]);

  useEffect(() => {
    if (authRequired) return;
    let stopped = false;
    let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
    const connect = async () => {
      try {
        const headers = new Headers();
        if (token) headers.set("Authorization", `Bearer ${token}`);
        const response = await fetch("/api/v1/events", { headers, cache: "no-store" });
        if (response.status === 401) { setAuthRequired(true); setConnected(false); return; }
        if (!response.ok || !response.body) throw new Error(`Live connection failed (${response.status})`);
        reader = response.body.getReader();
        setConnected(true);
        const decoder = new TextDecoder();
        let pending = "";
        while (!stopped) {
          const { done, value } = await reader.read();
          if (done) break;
          pending += decoder.decode(value, { stream: true });
          const blocks = pending.split("\n\n"); pending = blocks.pop() || "";
          for (const block of blocks) {
            const line = block.split("\n").find((row) => row.startsWith("data: "));
            if (line) {
              try { setOverview(JSON.parse(line.slice(6)) as Overview); setConnected(true); setError(""); window.dispatchEvent(new Event("admin-update")); }
              catch { /* Ignore one malformed/partial event and keep the stream alive. */ }
            }
          }
        }
      } catch (cause) {
        if (!stopped) { setConnected(false); setError(cause instanceof Error ? cause.message : String(cause)); }
      } finally {
        reader?.releaseLock();
        if (!stopped) window.setTimeout(connect, 2500);
      }
    };
    void connect();
    return () => { stopped = true; void reader?.cancel(); };
  }, [token, authRequired]);

  const actions = useMemo(() => ({
    request: <T,>(path: string, init?: RequestInit) => request<T>(path, token, init),
    refresh: () => setRevision((value) => value + 1),
  }), [token]);

  if (authRequired) return <Login onSubmit={(value) => setToken(value)} error={error} />;

  return <div className="shell">
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark">S</div><div><strong>SONO</strong><span>OPERATIONS</span></div></div>
      <div className="instance"><i className={connected ? "online" : "offline"} />{connected ? "Connected" : "Reconnecting"}<small>{health?.instance_id || "Application instance"}</small></div>
      <nav aria-label="Administration"><span className="nav-caption">PLATFORM</span>{nav.map((item) => <button key={item.id} className={page === item.id ? "nav active" : "nav"} onClick={() => setPage(item.id)}><span className="nav-icon">{item.icon}</span>{item.label}{item.id === "jobs" && overview?.jobs.active ? <b>{overview.jobs.active}</b> : null}</button>)}</nav>
      <div className="side-footer"><span>SONO-RENDERER</span><small>{health?.version || "—"}</small></div>
    </aside>
    <main className="main">
      <header className="topbar"><div><div className="eyebrow">PLATFORM CONTROL ROOM</div><h1>{nav.find((item) => item.id === page)?.label}</h1></div><div className="top-actions"><span className={connected ? "connection up" : "connection down"}><i />{connected ? "Live" : "Disconnected"}</span><button className="icon-button" onClick={actions.refresh} aria-label="Refresh data" title="Refresh">↻</button></div></header>
      {!connected && <div className="banner warn" role="status">{error || "Connection lost. Showing the last received state while reconnecting."}</div>}
      <section className="content">
        {page === "overview" && <OverviewPage overview={overview} health={health} actions={actions} />}
        {page === "jobs" && <JobsPage actions={actions} selected={selectedJob} setSelected={setSelectedJob} />}
        {page === "history" && <HistoryPage actions={actions} />}
        {page === "performance" && <PerformancePage actions={actions} />}
        {page === "cache" && <CachePage actions={actions} />}
        {page === "profiles" && <ProfilesPage actions={actions} />}
        {page === "diagnostics" && <DiagnosticsPage actions={actions} />}
        {page === "logs" && <LogsPage actions={actions} />}
        {page === "settings" && <SettingsPage actions={actions} />}
      </section>
      <footer className="footnote">One application instance · data from authoritative renderer services · {health ? `uptime ${duration(health.uptime_seconds)}` : "uptime unavailable"}</footer>
    </main>
  </div>;
}

function Login({ onSubmit, error }: { onSubmit: (token: string) => void; error: string }) {
  const [value, setValue] = useState("");
  return <main className="login-screen"><form className="login-card" onSubmit={(event) => { event.preventDefault(); onSubmit(value); }}>
    <div className="brand-mark large">S</div><div className="eyebrow">REMOTE ADMINISTRATION</div><h1>Authenticate</h1>
    <p>This instance requires its operator token. It stays in memory for this page session and is never written to browser storage.</p>
    <label htmlFor="admin-token">Admin token</label><input id="admin-token" type="password" autoComplete="current-password" autoFocus value={value} onChange={(event) => setValue(event.target.value)} />
    {error && <div className="error-text">{error}</div>}<button className="primary full" type="submit">Connect securely</button>
  </form></main>;
}

function OverviewPage({ overview, health, actions }: { overview: Overview | null; health: Health | null; actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T>; refresh: () => void } }) {
  if (!overview) return <Loading />;
  const j = overview.jobs;
  return <>
    <div className="welcome-row"><div><h2>System at a glance</h2><p>Live state from this Sono-Renderer instance.</p></div><button className="secondary" onClick={actions.refresh}>Refresh now</button></div>
    <div className="metric-grid five"><Metric label="Active work" value={j.active} tone={j.active ? "blue" : ""} note={j.paused ? "Queue paused" : "Queue running"} /><Metric label="Queued" value={j.queued} note="Waiting for a worker" /><Metric label="Completed" value={j.completed} tone="green" note="Retained queue entries" /><Metric label="Failed" value={j.failed} tone={j.failed ? "red" : ""} note="Retained queue entries" /><Metric label="Uptime" value={health ? duration(health.uptime_seconds) : "—"} note="This process" /></div>
    <div className="grid-two">
      <Panel title="Latest render measurement" subtitle="Authoritative Performance Laboratory record" action={<button className="text-button" onClick={() => location.hash = "performance"}>Open lab →</button>}>
        {overview.latest_performance ? <PerformanceSummary record={overview.latest_performance} /> : <Empty title="No performance records yet" text="A completed render will appear here when telemetry is available." />}
      </Panel>
      <Panel title="Host resources" subtitle="Only values the host currently exposes are shown">
        <div className="resource-grid"><Resource label="Process CPU" value={percent(overview.resources.process_cpu_percent)} note="Windows counter; needs two samples" /><Resource label="System CPU" value={percent(overview.resources.system_cpu_percent)} note="Windows host counter; needs two samples" /><Resource label="Process memory" value={bytes(overview.resources.process_memory_bytes)} note="Working set" /><Resource label="Process I/O" value={rate(overview.resources.process_io_bytes_per_second)} note="OS transfer bytes; not disk bandwidth" /><Resource label="GPU utilization" value={percent(overview.resources.gpu_utilization_percent)} note="Unavailable in this build" /><Resource label="VRAM" value={bytes(overview.resources.vram_bytes)} note="Unavailable in this build" /></div>
        <div className="muted small-text">{overview.resources.availability}</div>
      </Panel>
    </div>
    <Panel title="Recent failures" subtitle="From persistent render history">
      {overview.recent_failures.length ? <div className="table-wrap"><table><thead><tr><th>Attempt</th><th>Backend</th><th>When</th><th>What happened</th></tr></thead><tbody>{overview.recent_failures.map((record) => <tr key={record.id}><td className="mono">{shortId(record.id)}</td><td>{record.execution_backend}</td><td>{date(record.completed_unix_time_ms)}</td><td className="error-text">{record.error || "Render failed"}</td></tr>)}</tbody></table></div> : <Empty title="No recent failures" text="Failed attempts appear here without changing the job queue." />}
    </Panel>
  </>;
}

function JobsPage({ actions, selected, setSelected }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T>; refresh: () => void }; selected: string | null; setSelected: (id: string | null) => void }) {
  const [jobs, setJobs] = useState<Job[]>([]); const [detail, setDetail] = useState<Job | null>(null); const [error, setError] = useState(""); const [busy, setBusy] = useState(false);
  const load = useCallback(async () => { try { setJobs(await actions.request<Job[]>("/api/v1/jobs")); setError(""); } catch (e) { setError(message(e)); } }, [actions]);
  useEffect(() => { void load(); }, [load]);
  useEffect(() => { if (!selected) { setDetail(null); return; } void actions.request<Job>(`/api/v1/jobs/${encodeURIComponent(selected)}`).then(setDetail).catch((e) => setError(message(e))); }, [actions, selected]);
  useEffect(() => { const refresh = () => void load(); window.addEventListener("admin-update", refresh); return () => window.removeEventListener("admin-update", refresh); }, [load]);
  const mutate = async (path: string, confirm?: string) => { if (confirm && !window.confirm(confirm)) return; setBusy(true); try { await actions.request(path, { method: "POST" }); await load(); actions.refresh(); } catch (e) { setError(message(e)); } finally { setBusy(false); } };
  const job = detail?.id === selected ? detail : jobs.find((item) => item.id === selected);
  return <>
    <div className="welcome-row"><div><h2>Render queue</h2><p>Queue lifecycle and cancellation are handled by RenderJobManager.</p></div><div className="button-row"><button className="secondary" onClick={() => void mutate("/api/v1/queue/pause")}>Pause queue</button><button className="secondary" onClick={() => void mutate("/api/v1/queue/resume")}>Resume queue</button><button className="secondary" onClick={() => void load()}>Refresh</button></div></div>
    {error && <div className="banner error-banner">{error}</div>}
    <Panel title="Attempts" subtitle={`${jobs.length} retained queue entries`}><div className="table-wrap"><table><thead><tr><th>State</th><th>Attempt</th><th>Backend</th><th>Progress</th><th>Submitted</th><th>Output</th></tr></thead><tbody>{jobs.map((item) => <tr key={item.id} className={selected === item.id ? "selected-row" : "clickable-row"} onClick={() => setSelected(item.id)}><td><StateBadge state={item.state} /></td><td className="mono">{shortId(item.id)} · #{item.attempt}</td><td>{item.execution_backend}</td><td><Progress job={item} /></td><td>{date(item.submitted_unix_time_ms)}</td><td>{item.output_name || "—"}</td></tr>)}</tbody></table>{!jobs.length && <Empty title="Queue is empty" text="Jobs submitted from the native application and CLI appear here." />}</div></Panel>
    {job && <Panel title={`Job ${shortId(job.id)}`} subtitle={`${job.renderer_version} · ${job.profile_name || "current settings"}`} action={<button className="icon-button" onClick={() => setSelected(null)} aria-label="Close job details">×</button>}>
      <div className="detail-grid"><div><span>State</span><StateBadge state={job.state} /></div><div><span>Watch backend</span><b>{job.execution_backend}</b></div><div><span>Output</span><b>{job.output_name || "Not set"}</b></div><div><span>Failure type</span><b>{job.failure_kind || "—"}</b></div></div>
      {job.diagnostic && <div className="banner error-banner">{job.diagnostic}</div>}
      <div className="button-row">{["preparing", "compiling", "rendering", "encoding", "queued"].includes(job.state) && <button disabled={busy} className="danger" onClick={() => void mutate(`/api/v1/jobs/${encodeURIComponent(job.id)}/cancel`, "Request cancellation for this render?")}>Cancel render</button>}{job.state === "failed" && <button disabled={busy} className="primary" onClick={() => void mutate(`/api/v1/jobs/${encodeURIComponent(job.id)}/retry`, "Retry this failed render using its captured job configuration?")}>Retry failed render</button>}</div>
      {job.configuration_snapshot && <details className="config-detail"><summary>Captured effective configuration (secrets redacted)</summary><pre>{JSON.stringify(job.configuration_snapshot, null, 2)}</pre></details>}
      <h3>Recent job events</h3><div className="log-box">{job.logs.map((log, index) => <div key={`${log.unix_time_ms}-${index}`}><time>{date(log.unix_time_ms)}</time>{log.message}</div>)}</div>
    </Panel>}
  </>;
}

function HistoryPage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [records, setRecords] = useState<History[]>([]); const [query, setQuery] = useState(""); const [status, setStatus] = useState(""); const [error, setError] = useState("");
  useEffect(() => { const timer = window.setTimeout(() => { const params = new URLSearchParams(); if (query) params.set("text", query); if (status) params.set("status", status); params.set("limit", "100"); void actions.request<History[]>(`/api/v1/history?${params}`).then(setRecords).catch((e) => setError(message(e))); }, 180); return () => window.clearTimeout(timer); }, [actions, query, status]);
  return <><div className="welcome-row"><div><h2>Render history</h2><p>Durable attempt records; reruns create a new attempt.</p></div><div className="filter-row"><input aria-label="Search render history" placeholder="Search project, engine, output…" value={query} onChange={(e) => setQuery(e.target.value)} /><select aria-label="Filter by status" value={status} onChange={(e) => setStatus(e.target.value)}><option value="">All outcomes</option><option value="completed">Completed</option><option value="failed">Failed</option><option value="cancelled">Cancelled</option></select></div></div>{error && <div className="banner error-banner">{error}</div>}
    <Panel title="Attempts" subtitle={`${records.length} records`}><div className="table-wrap"><table><thead><tr><th>Outcome</th><th>Attempt</th><th>Backend</th><th>Profile</th><th>Finished</th><th>Output</th></tr></thead><tbody>{records.map((record) => <tr key={record.id}><td><StateBadge state={record.status} /></td><td className="mono">{shortId(record.id)}</td><td>{record.execution_backend}</td><td>{record.profile_name || "—"}</td><td>{date(record.completed_unix_time_ms)}</td><td>{record.output_name || "—"}</td></tr>)}</tbody></table>{!records.length && <Empty title="No matching attempts" text="Try another search or render a job first." />}</div></Panel>
  </>;
}

function PerformancePage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [records, setRecords] = useState<Performance[]>([]); const [left, setLeft] = useState(""); const [right, setRight] = useState(""); const [comparison, setComparison] = useState<any>(null); const [error, setError] = useState("");
  useEffect(() => { void actions.request<Performance[]>("/api/v1/performance?limit=100").then((values) => { setRecords(values); setLeft(values[1]?.id || ""); setRight(values[0]?.id || ""); }).catch((e) => setError(message(e))); }, [actions]);
  const compare = async () => { try { setComparison(await actions.request(`/api/v1/performance/compare?left=${encodeURIComponent(left)}&right=${encodeURIComponent(right)}`)); } catch (e) { setError(message(e)); } };
  return <><div className="welcome-row"><div><h2>Performance Lab</h2><p>Stage spans and effective backend come from the renderer's authoritative records.</p></div></div>{error && <div className="banner error-banner">{error}</div>}
    {comparison && <div className={comparison.comparable ? "banner" : "banner warn"}><strong>{comparison.classification}</strong> · {comparison.comparable ? `${formatMs(comparison.absolute_difference_ms)} (${formatPercent(comparison.percentage_difference)})` : `Not comparable: ${comparison.differences.join(", ")}`}</div>}
    <Panel title="Compare executions" subtitle="The backend enforces workload and environment comparability"><div className="compare-row"><select value={left} onChange={(e) => setLeft(e.target.value)}>{records.map((r) => <option key={r.id} value={r.id}>{shortId(r.id)} · {r.execution_mode} · {formatMs(r.execution_time_ms)}</option>)}</select><span>vs</span><select value={right} onChange={(e) => setRight(e.target.value)}>{records.map((r) => <option key={r.id} value={r.id}>{shortId(r.id)} · {r.execution_mode} · {formatMs(r.execution_time_ms)}</option>)}</select><button className="primary" disabled={!left || !right} onClick={() => void compare()}>Compare</button></div></Panel>
    {records.map((record) => <Panel key={record.id} title={`${formatMs(record.execution_time_ms)} execution`} subtitle={`${record.effective_backend_status} · ${record.resolution.join("×")} at ${record.fps} FPS · ${record.renderer_version}`}><PerformanceSummary record={record} detailed /></Panel>)}
    {!records.length && <Empty title="No measurements yet" text="Completed renderer runs write performance records when telemetry is available." />}
  </>;
}

function CachePage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [cache, setCache] = useState<Cache | null>(null); const [error, setError] = useState(""); const [result, setResult] = useState("");
  const load = useCallback(() => actions.request<Cache>("/api/v1/cache").then(setCache).catch((e) => setError(message(e))), [actions]);
  useEffect(() => { void load(); }, [load]);
  const clear = async () => { if (!window.confirm("Clear inactive generated caches? Projects, renders, profiles, and history are not removed.")) return; try { const report = await actions.request<any>("/api/v1/cache/clear", { method: "POST", body: JSON.stringify({ confirmation: "clear-generated-cache", include_memory: false }) }); setResult(`${report.removed_entries} entries removed · ${bytes(report.removed_bytes)} freed`); await load(); } catch (e) { setError(message(e)); } };
  return <><div className="welcome-row"><div><h2>Managed caches</h2><p>Opening this page only inspects. Cache removal uses the authoritative cache manager and keeps active entries protected.</p></div><div className="button-row"><button className="secondary" onClick={() => void load()}>Inspect</button><button className="secondary" onClick={() => void actions.request<Cache["disk_entries"]>("/api/v1/cache/validate", { method: "POST" }).then((entries) => { setResult(`${entries.length} entries validated; ${entries.filter((entry) => entry.status !== "valid").length} need attention.`); return load(); }).catch((cause) => setError(message(cause)))}>Validate entries</button><button className="danger" onClick={() => void clear()}>Clear inactive generated caches</button></div></div>{error && <div className="banner error-banner">{error}</div>}{result && <div className="banner">{result}</div>}
    {cache && <><div className="metric-grid"><Metric label="Managed disk caches" value={bytes(cache.total_disk_bytes)} note="Regenerable data only" /></div><Panel title="By category"><div className="category-list">{cache.regenerable_disk.map((item) => <div key={item.category}><div><b>{item.category}</b><span>{item.entries} entries</span></div><strong>{bytes(item.bytes)}</strong></div>)}</div></Panel><Panel title="Memory caches"><div className="category-list">{cache.memory.map((item) => <div key={item.category}><div><b>{item.category}</b><span>{item.entries} entries · {item.notes}</span></div><strong>{item.retained_bytes == null ? "Unavailable" : bytes(item.retained_bytes)}</strong></div>)}</div></Panel><Panel title="Inspectable native entries"><div className="table-wrap"><table><thead><tr><th>Category</th><th>Identifier</th><th>Size</th><th>Schema</th><th>Validity</th></tr></thead><tbody>{cache.disk_entries.map((item) => <tr key={item.id}><td>{item.category}</td><td className="mono">{shortId(item.id)}</td><td>{bytes(item.size_bytes)}</td><td>{item.schema ?? "—"}</td><td><StateBadge state={item.status} /></td></tr>)}</tbody></table></div></Panel></>}
  </>;
}

function ProfilesPage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [profiles, setProfiles] = useState<Profile[]>([]); const [error, setError] = useState("");
  const load = useCallback(() => actions.request<Profile[]>("/api/v1/profiles").then(setProfiles).catch((e) => setError(message(e))), [actions]);
  useEffect(() => { void load(); }, [load]);
  const select = async (id?: string) => { try { await actions.request("/api/v1/profiles/select", { method: "POST", body: JSON.stringify({ selector: id || "current" }) }); await load(); } catch (e) { setError(message(e)); } };
  const rename = async (profile: Profile) => { const name = window.prompt("New profile name", profile.name); if (!name || name === profile.name) return; try { await actions.request(`/api/v1/profiles/${encodeURIComponent(profile.id)}`, { method: "PATCH", body: JSON.stringify({ name }) }); await load(); } catch (e) { setError(message(e)); } };
  const remove = async (profile: Profile) => { if (!window.confirm(`Delete profile “${profile.name}”? Existing jobs keep their captured configuration.`)) return; try { await actions.request(`/api/v1/profiles/${encodeURIComponent(profile.id)}`, { method: "DELETE", body: JSON.stringify({ confirmation: "delete-profile" }) }); await load(); } catch (e) { setError(message(e)); } };
  return <><div className="welcome-row"><div><h2>Render profiles</h2><p>Profile selection changes future configuration. Existing jobs retain their captured snapshots.</p></div><button className="secondary" onClick={() => void select()}>Use current settings</button></div>{error && <div className="banner error-banner">{error}</div>}<div className="profile-grid">{profiles.map((profile) => <article className="profile-card" key={profile.id}><div className="profile-head"><span className="avatar">{profile.name.slice(0, 1).toUpperCase()}</span><div><h3>{profile.name}</h3><small>{profile.active ? "Active profile" : `Updated ${date(profile.updated_unix_time_ms)}`}</small></div><button className="icon-button" aria-label={`Rename ${profile.name}`} onClick={() => void rename(profile)}>···</button></div><div className="profile-facts"><span>{profile.resolution.join(" × ")}</span><span>{profile.fps} FPS</span><span>{profile.execution_mode}</span></div><div className="button-row">{!profile.active && <button className="primary" onClick={() => void select(profile.id)}>Make active</button>}<button className="text-button danger-text" onClick={() => void remove(profile)}>Delete</button></div></article>)}{!profiles.length && <Empty title="No saved profiles" text="The application currently uses its current/default render settings." />}</div></>;
}

function DiagnosticsPage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [items, setItems] = useState<Diagnostic[]>([]); const [error, setError] = useState("");
  useEffect(() => { void actions.request<Diagnostic[]>("/api/v1/diagnostics?limit=50").then(setItems).catch((e) => setError(message(e))); }, [actions]);
  return <><div className="welcome-row"><div><h2>Diagnostics</h2><p>Structured findings from render diagnostic records, not parsed terminal output.</p></div></div>{error && <div className="banner error-banner">{error}</div>}{items.map((item) => <Panel key={item.id} title={`${item.outcome} · ${shortId(item.id)}`} subtitle={`${item.effective_backend} · ${date(item.generated_unix_time_ms)}`}><ul className="finding-list">{item.findings.map((finding, index) => <li key={`${finding.code}-${index}`}><StateBadge state={finding.kind} /><span>{finding.summary}</span></li>)}</ul></Panel>)}{!items.length && !error && <Empty title="No diagnostic reports" text="Render failures and completed attempts can attach structured reports." />}</>;
}

function LogsPage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [items, setItems] = useState<LogEntry[]>([]); const [captureActive, setCaptureActive] = useState(false); const [retention, setRetention] = useState(0); const [filter, setFilter] = useState(""); const [level, setLevel] = useState(""); const [error, setError] = useState("");
  useEffect(() => {
    let active = true; let loading = false;
    const load = async () => {
      if (loading) return; loading = true;
      const params = new URLSearchParams(); if (filter) params.set("text", filter); if (level) params.set("level", level);
      try { const next = await actions.request<LogsResponse>(`/api/v1/logs?${params}`); if (active) { setItems(next.entries); setCaptureActive(next.application_capture_active); setRetention(next.application_retention_limit); setError(""); } }
      catch (e) { if (active) setError(message(e)); }
      finally { loading = false; }
    };
    void load(); const timer = window.setInterval(() => void load(), 5000);
    return () => { active = false; window.clearInterval(timer); };
  }, [actions, filter, level]);
  return <><div className="welcome-row"><div><h2>Application and job logs</h2><p>{captureActive ? `Application events are secret-redacted and retained in memory up to ${retention} entries for this process.` : "Application log capture is unavailable because this process is using another logger."} Render events remain attached to durable job history. Updates every five seconds while this page is open; process logs are cleared when the application exits.</p></div><div className="top-actions"><select value={level} onChange={(e) => setLevel(e.target.value)} aria-label="Filter log severity"><option value="">All severities</option><option value="error">Error</option><option value="warn">Warning</option><option value="info">Info</option></select><input placeholder="Filter logs…" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Filter application and job logs" /></div></div>{error && <div className="banner error-banner">{error}</div>}<div className="log-box large-log">{items.map((item, index) => <div key={`${item.source}-${item.job_id || item.target || "app"}-${item.unix_time_ms}-${index}`}><time>{date(item.unix_time_ms)}</time><StateBadge state={item.level || item.state || "event"} /><span className="muted">{item.source}</span>{item.job_id && <span className="mono">{shortId(item.job_id)}</span>}{item.target && <span className="mono muted">{item.target}</span>}{item.message}</div>)}{!items.length && <Empty title="No recent events" text="Application logs include Info, Warning, and Error events while capture is active. Render events remain attached to their jobs." />}</div></>;
}

function SettingsPage({ actions }: { actions: { request: <T,>(p: string, i?: RequestInit) => Promise<T> } }) {
  const [config, setConfig] = useState<AppConfig | null>(null); const [error, setError] = useState(""); const [saved, setSaved] = useState(false);
  useEffect(() => { void actions.request<AppConfig>("/api/v1/admin-config").then(setConfig).catch((e) => setError(message(e))); }, [actions]);
  const save = async () => { if (!config) return; try { const next = await actions.request<AppConfig>("/api/v1/admin-config", { method: "PUT", body: JSON.stringify({ enabled: config.enabled, bind_address: config.bind_address, port: config.port }) }); setConfig(next); setSaved(true); } catch (e) { setError(message(e)); } };
  if (!config) return <Loading />;
  return <><div className="welcome-row"><div><h2>Admin service</h2><p>Versioned platform settings. Binding changes take effect on the next application start.</p></div><button className="primary" onClick={() => void save()}>Save settings</button></div>{error && <div className="banner error-banner">{error}</div>}{saved && <div className="banner warn">Saved. Restart Sono-Renderer to apply these service settings.</div>}
    <Panel title="Service configuration" subtitle={`Authentication: ${config.authentication}`}><div className="form-grid"><label className="toggle-row"><input type="checkbox" checked={config.enabled} onChange={(e) => setConfig({ ...config, enabled: e.target.checked })} /><span><b>Enable hosted admin service</b><small>Starts the service with the native application when enabled.</small></span></label><label>Bind address<input value={config.bind_address} onChange={(e) => setConfig({ ...config, bind_address: e.target.value })} /><small>Loopback is the safe default. Remote binding requires TLS and the operator token.</small></label><label>Port<input type="number" min="0" max="65535" value={config.port} onChange={(e) => setConfig({ ...config, port: Number(e.target.value) })} /><small>Port 0 asks the operating system to select an available port.</small></label></div></Panel>
    <Panel title="Security model"><ul className="plain-list"><li>Loopback-only access uses the local operating-system account boundary by default.</li><li>Remote access requires HTTPS, a configured certificate/key, and an operator token from <code>SONO_RENDERER_ADMIN_TOKEN</code>.</li><li>The token is not saved in settings or browser storage.</li><li>Do not expose the service directly to the public Internet without an authenticated TLS deployment boundary.</li></ul></Panel>
  </>;
}

function PerformanceSummary({ record, detailed = false }: { record: Performance; detailed?: boolean }) {
  const shown = detailed ? record.stages : record.stages.slice(0, 5);
  return <><div className="perf-stats"><div><span>Execution</span><b>{formatMs(record.execution_time_ms)}</b></div><div><span>Queue wait</span><b>{record.queue_wait_time_ms == null ? "—" : formatMs(record.queue_wait_time_ms)}</b></div><div><span>Effective backend</span><b>{record.effective_backend_status}</b></div><div><span>Cache</span><b>{record.cache_state || "Not reported"}</b></div></div><div className="stage-list">{shown.map((stage) => <div className="stage" key={stage.name}><span>{stage.name}<small>{stage.semantics} · {stage.samples} samples</small></span><div className="stage-track"><i style={{ width: `${Math.min(100, record.execution_time_ms ? stage.duration_ms / record.execution_time_ms * 100 : 0)}%` }} /></div><b>{formatMs(stage.duration_ms)}</b></div>)}{!record.stages.length && <span className="muted">Stage data unavailable for this record.</span>}</div><div className="muted small-text">{record.environment.operating_system} · {record.environment.architecture}{record.environment.cpu_identity ? ` · ${record.environment.cpu_identity}` : ""}{record.environment.gpu_identity ? ` · ${record.environment.gpu_identity}` : ""}</div></>;
}
function Metric({ label, value, note, tone = "" }: { label: string; value: React.ReactNode; note: string; tone?: string }) { return <article className="metric"><span>{label}</span><strong className={tone}>{value}</strong><small>{note}</small></article>; }
function Resource({ label, value, note }: { label: string; value: string; note: string }) { return <div className="resource"><span>{label}</span><b>{value}</b><small>{note}</small></div>; }
function Panel({ title, subtitle, action, children }: { title: string; subtitle?: string; action?: React.ReactNode; children: React.ReactNode }) { return <article className="panel"><div className="panel-head"><div><h3>{title}</h3>{subtitle && <p>{subtitle}</p>}</div>{action}</div>{children}</article>; }
function Empty({ title, text }: { title: string; text: string }) { return <div className="empty"><span>◌</span><b>{title}</b><p>{text}</p></div>; }
function Loading() { return <div className="loading"><i /> Loading platform state…</div>; }
function StateBadge({ state }: { state: string }) { return <span className={`badge ${state.toLowerCase().replaceAll("_", "-")}`}>{state.replaceAll("_", " ")}</span>; }
function Progress({ job }: { job: Job }) { return <div className="progress-cell"><div className="progress-track"><i style={{ width: `${Math.max(0, Math.min(100, job.percentage || 0))}%` }} /></div><small>{job.percentage == null ? job.phase || job.state : `${job.percentage.toFixed(1)}%`}</small></div>; }
function duration(seconds: number) { if (seconds < 60) return `${Math.floor(seconds)}s`; if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`; return `${Math.floor(seconds / 3600)}h ${Math.floor(seconds % 3600 / 60)}m`; }
function date(value?: number) { return value ? new Date(value).toLocaleString() : "—"; }
function bytes(value: number | null) { if (value == null) return "Unavailable"; const units = ["B", "KB", "MB", "GB", "TB"]; let n = value; let i = 0; while (n >= 1024 && i < units.length - 1) { n /= 1024; i++; } return `${n.toFixed(i ? 1 : 0)} ${units[i]}`; }
function rate(value: number | null) { return value == null ? "Unavailable" : `${bytes(value)}/s`; }
function percent(value: number | null) { return value == null ? "Unavailable" : `${value.toFixed(1)}%`; }
function formatMs(value?: number | null) { return value == null ? "—" : value >= 1000 ? `${(value / 1000).toFixed(2)} s` : `${value.toFixed(1)} ms`; }
function formatPercent(value?: number | null) { return value == null ? "—" : `${value > 0 ? "+" : ""}${value.toFixed(1)}%`; }
function shortId(value: string) { return value.length > 18 ? `${value.slice(0, 9)}…${value.slice(-6)}` : value; }
function message(error: unknown) { return error instanceof Error ? error.message : String(error); }

createRoot(document.getElementById("root")!).render(<React.StrictMode><App /></React.StrictMode>);
