/* SMOS Dashboard — hash-router SPA */
(function () {
  const $ = (sel, el = document) => el.querySelector(sel);
  const $$ = (sel, el = document) => [...el.querySelectorAll(sel)];

  const state = {
    route: "overview",
    health: null,
    metrics: null,
    metricsHistory: null,
    historyRangeHours: 24,
    historyStatus: null,
    processes: [],
    logs: [],
    logTail: null,
    config: null,
    audit: [],
    cpuHistory: [],
    memHistory: [],
    pollTimer: null,
    error: null,
  };

  function token() {
    return localStorage.getItem("smos_token") || $("#auth-token")?.value || "";
  }

  function saveToken(v) {
    if (v) localStorage.setItem("smos_token", v);
    else localStorage.removeItem("smos_token");
  }

  async function api(path, opts = {}) {
    const headers = Object.assign({ Accept: "application/json" }, opts.headers || {});
    if (opts.body && !headers["Content-Type"]) {
      headers["Content-Type"] = "application/json";
    }
    const t = token();
    if (t) headers["X-SMOS-Token"] = t;
    const res = await fetch("/api" + path, { ...opts, headers });
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { data = { raw: text }; }
    if (!res.ok) {
      const msg = (data && data.error) || res.statusText || "request failed";
      throw new Error(msg);
    }
    return data;
  }

  function fmtBytes(n) {
    if (n == null || isNaN(n)) return "—";
    const u = ["B", "KB", "MB", "GB", "TB"];
    let i = 0;
    let v = Number(n);
    while (v >= 1024 && i < u.length - 1) { v /= 1024; i++; }
    return v.toFixed(v >= 10 || i === 0 ? 0 : 1) + " " + u[i];
  }

  function fmtPct(n) {
    if (n == null || isNaN(n)) return "—";
    return Number(n).toFixed(1) + "%";
  }

  function fmtUptime(secs) {
    secs = Math.max(0, Number(secs) || 0);
    const d = Math.floor(secs / 86400);
    const h = Math.floor((secs % 86400) / 3600);
    const m = Math.floor((secs % 3600) / 60);
    if (d > 0) return `${d}d ${h}h ${m}m`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  }

  function barClass(pct) {
    if (pct >= 90) return "bar danger";
    if (pct >= 75) return "bar warn";
    return "bar";
  }

  function spark(history) {
    const max = Math.max(1, ...history, 1);
    return `<div class="spark">${history.map(v => {
      const h = Math.max(2, Math.round((v / max) * 100));
      return `<i style="height:${h}%"></i>`;
    }).join("")}</div>`;
  }

  function setHealthPill(ok, text) {
    const el = $("#health-pill");
    el.textContent = text;
    el.className = "pill " + (ok ? "ok" : "bad");
  }

  function setActiveNav(route) {
    $$(".nav-item").forEach(a => {
      a.classList.toggle("active", a.dataset.route === route);
    });
  }

  const titles = {
    overview: "Overview",
    metrics: "Performance",
    processes: "Processes",
    logs: "Logs",
    config: "Configuration",
    audit: "Audit Journal",
  };

  function render() {
    const view = $("#view");
    $("#page-title").textContent = titles[state.route] || "SMOS";
    setActiveNav(state.route);
    if (state.health) {
      $("#host-label").textContent = state.health.host_label || "smos-host";
      $("#version").textContent = "v" + (state.health.version || "?");
    }
    try {
      switch (state.route) {
        case "metrics": view.innerHTML = renderMetrics(); break;
        case "processes": view.innerHTML = renderProcesses(); break;
        case "logs": view.innerHTML = renderLogs(); break;
        case "config": view.innerHTML = renderConfig(); break;
        case "audit": view.innerHTML = renderAudit(); break;
        default: view.innerHTML = renderOverview();
      }
      bindViewHandlers();
    } catch (e) {
      view.innerHTML = `<div class="msg err">${esc(e.message)}</div>`;
    }
  }

  function esc(s) {
    return String(s ?? "")
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;");
  }

  function renderOverview() {
    const m = state.metrics;
    const h = state.health;
    if (!m) return `<div class="empty">Loading metrics…</div>`;
    const cpu = m.cpu?.usage_percent ?? 0;
    const mem = m.memory?.usage_percent ?? 0;
    const disk = (m.disks && m.disks[0]) || null;
    return `
      <div class="grid">
        <div class="card">
          <h3>CPU</h3>
          <div class="metric-value">${fmtPct(cpu)}</div>
          <div class="metric-sub">${m.cpu?.core_count || 0} cores · ${esc(m.cpu?.brand || "")}</div>
          <div class="${barClass(cpu)}"><span style="width:${Math.min(100, cpu)}%"></span></div>
          ${spark(state.cpuHistory)}
        </div>
        <div class="card">
          <h3>Memory</h3>
          <div class="metric-value">${fmtPct(mem)}</div>
          <div class="metric-sub">${fmtBytes(m.memory?.used_bytes)} / ${fmtBytes(m.memory?.total_bytes)}</div>
          <div class="${barClass(mem)}"><span style="width:${Math.min(100, mem)}%"></span></div>
          ${spark(state.memHistory)}
        </div>
        <div class="card">
          <h3>Primary disk</h3>
          <div class="metric-value">${disk ? fmtPct(disk.usage_percent) : "—"}</div>
          <div class="metric-sub">${disk ? esc(disk.mount_point) + " · " + fmtBytes(disk.used_bytes) + " / " + fmtBytes(disk.total_bytes) : "No disks"}</div>
          <div class="${barClass(disk?.usage_percent || 0)}"><span style="width:${Math.min(100, disk?.usage_percent || 0)}%"></span></div>
        </div>
        <div class="card wide">
          <h3>Host</h3>
          <table>
            <tr><th>Hostname</th><td class="mono">${esc(m.hostname)}</td></tr>
            <tr><th>Label</th><td class="mono">${esc(h?.host_label || "—")}</td></tr>
            <tr><th>Host uptime</th><td class="mono">${fmtUptime(m.uptime_secs)}</td></tr>
            <tr><th>SMOS uptime</th><td class="mono">${fmtUptime(h?.uptime_secs)}</td></tr>
            <tr><th>Processes</th><td class="mono">${state.processes.length}</td></tr>
          </table>
        </div>
        <div class="card">
          <h3>Quick links</h3>
          <div style="display:grid;gap:0.5rem">
            <a class="btn secondary" href="#/processes">Manage processes</a>
            <a class="btn secondary" href="#/logs">Browse logs</a>
            <a class="btn secondary" href="#/audit">Audit journal</a>
          </div>
        </div>
      </div>`;
  }

  function historySeries(key) {
    const samples = state.metricsHistory?.samples || [];
    return samples.map(s => Number(s[key]) || 0);
  }

  function renderHistoryToolbar() {
    const hours = state.historyRangeHours || 24;
    const opts = [
      [1, "1h"], [6, "6h"], [24, "24h"], [72, "3d"], [168, "7d"], [720, "30d"],
    ];
    const buttons = opts.map(([h, label]) =>
      `<button type="button" class="btn ${hours === h ? "" : "secondary"}" data-hist-hours="${h}">${label}</button>`
    ).join(" ");
    const count = state.metricsHistory?.count ?? 0;
    const ret = state.metricsHistory?.retention_days
      ?? state.config?.history_retention_days
      ?? 30;
    return `
      <div class="card full">
        <h3>Stored history</h3>
        <p class="muted">Samples are written every ${state.config?.metrics_history_interval_secs || 60}s and kept for <strong>${ret} days</strong> (change under Config).</p>
        <div class="log-toolbar" style="flex-wrap:wrap;gap:0.4rem">${buttons}
          <span class="muted" style="margin-left:0.5rem">${count} points · range ${hours}h</span>
        </div>
        <div class="grid" style="margin-top:1rem">
          <div class="card">
            <h3>CPU history</h3>
            ${spark(historySeries("cpu").length ? historySeries("cpu") : state.cpuHistory)}
          </div>
          <div class="card">
            <h3>Memory history</h3>
            ${spark(historySeries("mem").length ? historySeries("mem") : state.memHistory)}
          </div>
        </div>
      </div>`;
  }

  function renderMetrics() {
    const m = state.metrics;
    if (!m) return `<div class="empty">Loading…</div>`;
    const cores = (m.cpu?.per_core || []).map((c, i) =>
      `<tr><td class="mono">core-${i}</td><td class="mono">${fmtPct(c)}</td>
       <td><div class="${barClass(c)}"><span style="width:${Math.min(100, c)}%"></span></div></td></tr>`
    ).join("");
    const disks = (m.disks || []).map(d =>
      `<tr>
        <td class="mono">${esc(d.mount_point)}</td>
        <td>${esc(d.file_system)}</td>
        <td class="mono">${fmtBytes(d.used_bytes)} / ${fmtBytes(d.total_bytes)}</td>
        <td class="mono">${fmtPct(d.usage_percent)}</td>
      </tr>`
    ).join("");
    return `
      <div class="grid">
        ${renderHistoryToolbar()}
        <div class="card">
          <h3>CPU global</h3>
          <div class="metric-value">${fmtPct(m.cpu.usage_percent)}</div>
          <div class="metric-sub">${m.cpu.core_count} cores</div>
          ${spark(state.cpuHistory)}
        </div>
        <div class="card">
          <h3>Memory</h3>
          <div class="metric-value">${fmtPct(m.memory.usage_percent)}</div>
          <div class="metric-sub">Avail ${fmtBytes(m.memory.available_bytes)}</div>
          ${spark(state.memHistory)}
        </div>
        <div class="card">
          <h3>Swap</h3>
          <div class="metric-value">${fmtBytes(m.memory.swap_used_bytes)}</div>
          <div class="metric-sub">of ${fmtBytes(m.memory.swap_total_bytes)}</div>
        </div>
        <div class="card wide">
          <h3>Per-core CPU</h3>
          <table><thead><tr><th>Core</th><th>Usage</th><th></th></tr></thead><tbody>${cores || '<tr><td colspan="3" class="empty">No core data</td></tr>'}</tbody></table>
        </div>
        <div class="card">
          <h3>Load average</h3>
          <div class="metric-sub mono">${m.load_avg ? `${m.load_avg.one.toFixed(2)} / ${m.load_avg.five.toFixed(2)} / ${m.load_avg.fifteen.toFixed(2)}` : "n/a on this OS"}</div>
        </div>
        <div class="card full">
          <h3>Disks</h3>
          <table>
            <thead><tr><th>Mount</th><th>FS</th><th>Used</th><th>%</th></tr></thead>
            <tbody>${disks || '<tr><td colspan="4" class="empty">No disks</td></tr>'}</tbody>
          </table>
        </div>
      </div>`;
  }

  function renderProcesses() {
    const rows = (state.processes || []).slice(0, 150).map(p => `
      <tr>
        <td class="mono">${p.pid}</td>
        <td>${esc(p.name)}</td>
        <td class="mono">${fmtPct(p.cpu_usage)}</td>
        <td class="mono">${fmtBytes(p.memory_bytes)}</td>
        <td class="mono">${esc(p.status)}</td>
        <td class="row-actions">
          <button class="btn secondary" data-act="terminate" data-pid="${p.pid}">Term</button>
          <button class="btn danger" data-act="kill" data-pid="${p.pid}">Kill</button>
        </td>
      </tr>
      <tr><td></td><td colspan="5" class="mono muted" style="padding-top:0;border:0">${esc((p.cmd || p.exe || "").slice(0, 160))}</td></tr>
    `).join("");
    return `
      <div class="grid">
        <div class="card full">
          <h3>Running processes (${state.processes.length})</h3>
          <p class="muted">Terminate sends a graceful signal; Kill forces exit. Self-process actions are blocked.</p>
          <div style="overflow:auto;max-height:70vh">
            <table>
              <thead><tr><th>PID</th><th>Name</th><th>CPU</th><th>Mem</th><th>Status</th><th>Actions</th></tr></thead>
              <tbody>${rows || '<tr><td colspan="6" class="empty">No processes</td></tr>'}</tbody>
            </table>
          </div>
        </div>
      </div>`;
  }

  function renderLogs() {
    const sources = (state.logs || []).map(s =>
      `<option value="${esc(s.id)}" ${state.logTail && state.logTail.source_id === s.id ? "selected" : ""}>${esc(s.label)} ${s.exists ? "" : "(missing)"}</option>`
    ).join("");
    const lines = (state.logTail?.lines || []).map(l => esc(l)).join("\n");
    const ret = state.config?.history_retention_days ?? 30;
    const histFiles = state.historyStatus?.log_files || [];
    const histNote = histFiles.length
      ? `${histFiles.length} log file(s) on disk · ${fmtBytes(state.historyStatus.log_bytes_total || 0)}`
      : "Daily log files appear under data dir as smos.log.YYYY-MM-DD";
    return `
      <div class="grid">
        <div class="card full">
          <h3>Log sources</h3>
          <p class="muted">Service logs rotate daily and are retained for <strong>${ret} days</strong>. Older days appear as History sources below.</p>
          <div class="log-toolbar">
            <select id="log-source">${sources || '<option value="smos-service">smos-service</option>'}</select>
            <label class="muted">Lines <input id="log-lines" type="number" min="10" max="5000" value="${state.logTail?.line_count || 200}" style="width:90px" /></label>
            <button class="btn" id="log-load">Load</button>
          </div>
          <div class="log-box" id="log-box">${lines || "No log lines yet."}</div>
          <div class="muted" style="margin-top:0.5rem">${state.logTail ? esc(state.logTail.path) + " · " + state.logTail.line_count + " lines" : ""} · ${esc(histNote)}</div>
        </div>
      </div>`;
  }

  function renderConfig() {
    const c = state.config;
    if (!c) return `<div class="empty">Loading config…</div>`;
    const hs = state.historyStatus;
    return `
      <div class="grid">
        <div class="card wide">
          <h3>SMOS configuration</h3>
          <form id="config-form" class="form-grid">
            <label>Host label<input name="host_label" value="${esc(c.host_label)}" required maxlength="128" /></label>
            <label>Bind address<input name="bind" value="${esc(c.bind)}" required /></label>
            <label>Log tail lines<input name="log_tail_lines" type="number" min="1" max="50000" value="${c.log_tail_lines}" /></label>
            <label>Metrics poll (sec)<input name="metrics_poll_secs" type="number" min="1" max="3600" value="${c.metrics_poll_secs}" /></label>
            <label>History retention (days)<input name="history_retention_days" type="number" min="1" max="3650" value="${c.history_retention_days ?? 30}" /></label>
            <label>Metrics history interval (sec)<input name="metrics_history_interval_secs" type="number" min="10" max="3600" value="${c.metrics_history_interval_secs ?? 60}" /></label>
            <label class="full">Data dir<input value="${esc(c.data_dir)}" disabled /></label>
            <label class="full">Auth token set: <strong>${c.auth_token_set ? "yes" : "no"}</strong> (set via CLI/env SMOS_AUTH_TOKEN)</label>
            <div class="form-actions full">
              <button class="btn" type="submit">Save changes</button>
            </div>
          </form>
          <div id="config-msg"></div>
        </div>
        <div class="card">
          <h3>History storage</h3>
          <p class="muted">Default retention is <strong>30 days</strong>. Increase retention here to keep metrics and rotated logs longer. Interval changes apply to new samples; prune runs hourly.</p>
          ${hs ? `<table>
            <tr><th>Metrics samples</th><td class="mono">${hs.metrics_samples_on_disk}</td></tr>
            <tr><th>Metrics file size</th><td class="mono">${fmtBytes(hs.metrics_bytes)}</td></tr>
            <tr><th>Log files</th><td class="mono">${(hs.log_files || []).length} · ${fmtBytes(hs.log_bytes_total)}</td></tr>
            <tr><th>Oldest sample</th><td class="mono">${esc(hs.metrics_oldest || "—")}</td></tr>
          </table>` : `<p class="muted">Loading history status…</p>`}
        </div>
        <div class="card">
          <h3>Notes</h3>
          <p class="muted">Invalid values are rejected by the API. Successful updates are written to <span class="mono">config.json</span> and recorded in the audit journal. Bind changes apply on next restart.</p>
        </div>
      </div>`;
  }

  function renderAudit() {
    const rows = (state.audit || []).map(e => `
      <tr>
        <td class="mono">${esc(e.timestamp)}</td>
        <td class="mono">${esc(e.action)}</td>
        <td>${esc(e.target)}</td>
        <td>${e.success ? '<span style="color:var(--ok)">ok</span>' : '<span style="color:var(--danger)">fail</span>'}</td>
        <td class="mono">${esc(JSON.stringify(e.detail).slice(0, 120))}</td>
      </tr>
    `).join("");
    return `
      <div class="grid">
        <div class="card full">
          <h3>Audit journal</h3>
          <table>
            <thead><tr><th>Time</th><th>Action</th><th>Target</th><th>Result</th><th>Detail</th></tr></thead>
            <tbody>${rows || '<tr><td colspan="5" class="empty">No audit entries yet</td></tr>'}</tbody>
          </table>
        </div>
      </div>`;
  }

  function bindViewHandlers() {
    $$("[data-act]").forEach(btn => {
      btn.addEventListener("click", async () => {
        const pid = Number(btn.dataset.pid);
        const action = btn.dataset.act;
        if (!confirm(`${action} process ${pid}?`)) return;
        btn.disabled = true;
        try {
          await api(`/processes/${pid}/action`, {
            method: "POST",
            body: JSON.stringify({ action }),
          });
          await loadProcesses();
          await loadAudit();
          render();
        } catch (e) {
          alert("Action failed: " + e.message);
          btn.disabled = false;
        }
      });
    });

    const logLoad = $("#log-load");
    if (logLoad) {
      logLoad.addEventListener("click", async () => {
        const id = $("#log-source").value;
        const lines = Number($("#log-lines").value) || 200;
        try {
          state.logTail = await api(`/logs/${encodeURIComponent(id)}?lines=${lines}`);
          render();
        } catch (e) {
          alert(e.message);
        }
      });
    }

    $$("[data-hist-hours]").forEach(btn => {
      btn.addEventListener("click", async () => {
        state.historyRangeHours = Number(btn.dataset.histHours) || 24;
        try {
          await loadMetricsHistory();
          render();
        } catch (e) {
          alert(e.message);
        }
      });
    });

    const form = $("#config-form");
    if (form) {
      form.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        const fd = new FormData(form);
        const body = {
          host_label: fd.get("host_label"),
          bind: fd.get("bind"),
          log_tail_lines: Number(fd.get("log_tail_lines")),
          metrics_poll_secs: Number(fd.get("metrics_poll_secs")),
          history_retention_days: Number(fd.get("history_retention_days")),
          metrics_history_interval_secs: Number(fd.get("metrics_history_interval_secs")),
        };
        const msg = $("#config-msg");
        try {
          state.config = await api("/config", { method: "PUT", body: JSON.stringify(body) });
          msg.className = "msg ok";
          msg.textContent = "Configuration saved.";
          await loadAudit();
          await loadHealth();
          await loadHistoryStatus();
        } catch (e) {
          msg.className = "msg err";
          msg.textContent = e.message;
        }
      });
    }
  }

  async function loadHealth() {
    state.health = await api("/health");
    setHealthPill(true, "online");
  }

  async function loadMetrics() {
    state.metrics = await api("/metrics");
    const cpu = state.metrics.cpu?.usage_percent ?? 0;
    const mem = state.metrics.memory?.usage_percent ?? 0;
    state.cpuHistory.push(cpu);
    state.memHistory.push(mem);
    if (state.cpuHistory.length > 40) state.cpuHistory.shift();
    if (state.memHistory.length > 40) state.memHistory.shift();
  }

  async function loadMetricsHistory() {
    const hours = state.historyRangeHours || 24;
    state.metricsHistory = await api(`/metrics/history?hours=${hours}&limit=500`);
  }

  async function loadHistoryStatus() {
    state.historyStatus = await api("/history");
  }

  async function loadProcesses() {
    state.processes = await api("/processes");
  }

  async function loadLogs() {
    state.logs = await api("/logs");
    try { await loadHistoryStatus(); } catch { /* optional */ }
    if (!state.logTail && state.logs[0]) {
      state.logTail = await api(`/logs/${encodeURIComponent(state.logs[0].id)}?lines=200`);
    }
  }

  async function loadConfig() {
    state.config = await api("/config");
    try { await loadHistoryStatus(); } catch { /* optional */ }
  }

  async function loadAudit() {
    state.audit = await api("/audit?limit=100");
  }

  async function refreshAll() {
    try {
      await loadHealth();
      await loadMetrics();
      if (state.route === "processes" || state.route === "overview") await loadProcesses();
      if (state.route === "metrics" || state.route === "overview") {
        try { await loadMetricsHistory(); } catch { /* empty history ok */ }
      }
      if (state.route === "logs") await loadLogs();
      if (state.route === "config") await loadConfig();
      if (state.route === "audit") await loadAudit();
      render();
    } catch (e) {
      setHealthPill(false, "offline");
      state.error = e.message;
      $("#view").innerHTML = `<div class="msg err">API error: ${esc(e.message)}</div>`;
    }
  }

  function parseRoute() {
    const hash = location.hash.replace(/^#\/?/, "") || "overview";
    const route = hash.split("?")[0].split("/")[0] || "overview";
    state.route = titles[route] ? route : "overview";
  }

  function onRoute() {
    parseRoute();
    refreshAll();
  }

  function startPoll() {
    if (state.pollTimer) clearInterval(state.pollTimer);
    const secs = state.config?.metrics_poll_secs || 2;
    state.pollTimer = setInterval(async () => {
      if (document.hidden) return;
      try {
        await loadMetrics();
        if (state.route === "overview" || state.route === "metrics") render();
        if (state.route === "overview") {
          await loadHealth();
        }
      } catch { /* keep last good data */ }
    }, Math.max(1, secs) * 1000);
  }

  // boot
  const tokenInput = $("#auth-token");
  tokenInput.value = localStorage.getItem("smos_token") || "";
  tokenInput.addEventListener("change", () => saveToken(tokenInput.value.trim()));
  $("#refresh-btn").addEventListener("click", () => refreshAll());
  window.addEventListener("hashchange", onRoute);

  (async function init() {
    parseRoute();
    try {
      await loadHealth();
      await loadMetrics();
      await loadProcesses();
      await loadConfig();
      startPoll();
      // preload route-specific
      if (state.route === "logs") await loadLogs();
      if (state.route === "audit") await loadAudit();
      render();
    } catch (e) {
      setHealthPill(false, "offline");
      $("#view").innerHTML = `<div class="msg err">Failed to connect to SMOS API: ${esc(e.message)}</div>`;
    }
  })();
})();
