/* SMOS Dashboard — hash-router SPA */
(function () {
  const $ = (sel, el = document) => el.querySelector(sel);
  const $$ = (sel, el = document) => [...el.querySelectorAll(sel)];

  const state = {
    route: "overview",
    health: null,
    auth: null,
    metrics: null,
    metricsHistory: null,
    historyRangeHours: 24,
    historyStatus: null,
    processes: [],
    processFilter: { name: "", sort: "cpu", order: "desc" },
    logs: [],
    logTail: null,
    config: null,
    alerts: null,
    audit: [],
    cpuHistory: [],
    memHistory: [],
    pollTimer: null,
    error: null,
    // Auth gate phase: null | "setup" | "login" | "totp" | "totp_enroll"
    gate: null,
    pendingToken: null,
    pendingEmail: null,
    totpEnroll: null,
  };

  function sessionToken() {
    return localStorage.getItem("smos_session") || "";
  }

  function saveSession(v) {
    if (v) localStorage.setItem("smos_session", v);
    else localStorage.removeItem("smos_session");
  }

  async function api(path, opts = {}) {
    const headers = Object.assign({ Accept: "application/json" }, opts.headers || {});
    if (opts.body && !headers["Content-Type"]) {
      headers["Content-Type"] = "application/json";
    }
    const t = sessionToken();
    if (t) {
      headers["X-SMOS-Session"] = t;
      headers["Authorization"] = "Bearer " + t;
    }
    const res = await fetch("/api" + path, { ...opts, headers });
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { data = { raw: text }; }
    if (res.status === 401 || res.status === 403) {
      const msg = (data && data.error) || res.statusText || "unauthorized";
      const err = new Error(msg);
      err.status = res.status;
      err.setupRequired = res.status === 403;
      throw err;
    }
    if (!res.ok) {
      const msg = (data && data.error) || res.statusText || "request failed";
      throw new Error(msg);
    }
    return data;
  }

  function showApp(show) {
    $("#app").classList.toggle("hidden", !show);
    $("#auth-gate").classList.toggle("hidden", show);
    if (!show) closeNav();
  }

  function openNav() {
    document.body.classList.add("nav-open");
    const btn = $("#nav-open");
    if (btn) btn.setAttribute("aria-expanded", "true");
    const backdrop = $("#nav-backdrop");
    if (backdrop) backdrop.hidden = false;
  }

  function closeNav() {
    document.body.classList.remove("nav-open");
    const btn = $("#nav-open");
    if (btn) btn.setAttribute("aria-expanded", "false");
    const backdrop = $("#nav-backdrop");
    if (backdrop) backdrop.hidden = true;
  }

  function toggleNav() {
    if (document.body.classList.contains("nav-open")) closeNav();
    else openNav();
  }

  function renderGate() {
    const gate = $("#auth-gate");
    showApp(false);
    if (state.gate === "setup") {
      gate.innerHTML = `
        <div class="auth-card wide">
          <div class="auth-steps"><span class="on">1 · Account</span><span>2 · Optional 2FA</span><span>3 · Dashboard</span></div>
          <h1>Welcome to SMOS</h1>
          <p class="lead">First-time setup. Create the operator account for this host. You will use this email and password to sign in.</p>
          <form id="setup-form">
            <label>Email<input name="email" type="email" required autocomplete="username" placeholder="admin@example.com" /></label>
            <label>Password<input name="password" type="password" required minlength="8" autocomplete="new-password" placeholder="min. 8 characters" /></label>
            <label>Confirm password<input name="password2" type="password" required minlength="8" autocomplete="new-password" /></label>
            <label class="check">
              <input name="enable_totp" type="checkbox" />
              <span>Also set up offline OTP 2FA (authenticator app — codes work without Wi‑Fi after setup)</span>
            </label>
            <button class="btn" type="submit">Create account</button>
            <div id="gate-msg"></div>
          </form>
        </div>`;
      $("#setup-form").addEventListener("submit", onSetupSubmit);
      return;
    }
    if (state.gate === "totp_enroll") {
      const t = state.totpEnroll || {};
      gate.innerHTML = `
        <div class="auth-card wide">
          <div class="auth-steps"><span>1 · Account</span><span class="on">2 · Offline 2FA</span><span>3 · Dashboard</span></div>
          <h1>Enable authenticator (TOTP)</h1>
          <p class="lead">${esc(t.note || "Add this secret to Google Authenticator, Aegis, or Authy. Codes are generated offline — no Wi‑Fi needed.")}</p>
          <p class="muted">Account: <span class="mono">${esc(t.account || "")}</span></p>
          <p class="muted">Secret (manual entry)</p>
          <div class="secret-box">${esc(t.secret || "")}</div>
          <p class="muted" style="margin-top:0.75rem">otpauth URI</p>
          <div class="secret-box" style="font-size:0.75rem">${esc(t.otpauth_url || "")}</div>
          <form id="enroll-form" style="margin-top:1rem">
            <label>Enter 6-digit code from your app to confirm
              <input class="otp-input" name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required autocomplete="one-time-code" />
            </label>
            <div class="auth-actions">
              <button class="btn" type="submit">Verify & enable 2FA</button>
              <button class="btn secondary" type="button" id="skip-totp">Skip for now</button>
            </div>
            <div id="gate-msg"></div>
          </form>
        </div>`;
      $("#enroll-form").addEventListener("submit", onEnrollSubmit);
      $("#skip-totp").addEventListener("click", () => enterDashboard());
      return;
    }
    if (state.gate === "totp") {
      gate.innerHTML = `
        <div class="auth-card">
          <h1>Two-factor code</h1>
          <p class="lead">Enter the 6-digit code from your authenticator app. Works offline — no Wi‑Fi required.</p>
          <p class="muted">${esc(state.pendingEmail || "")}</p>
          <form id="totp-form">
            <label>OTP code
              <input class="otp-input" name="code" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required autocomplete="one-time-code" />
            </label>
            <div class="auth-actions">
              <button class="btn" type="submit">Verify</button>
              <button class="btn secondary" type="button" id="back-login">Back</button>
            </div>
            <div id="gate-msg"></div>
          </form>
        </div>`;
      $("#totp-form").addEventListener("submit", onTotpSubmit);
      $("#back-login").addEventListener("click", () => {
        state.gate = "login";
        state.pendingToken = null;
        renderGate();
      });
      return;
    }
    // login
    gate.innerHTML = `
      <div class="auth-card">
        <h1>Sign in to SMOS</h1>
        <p class="lead">Use the operator email and password created during onboarding.</p>
        <form id="login-form">
          <label>Email<input name="email" type="email" required autocomplete="username" /></label>
          <label>Password<input name="password" type="password" required autocomplete="current-password" /></label>
          <button class="btn" type="submit">Sign in</button>
          <div id="gate-msg"></div>
        </form>
      </div>`;
    $("#login-form").addEventListener("submit", onLoginSubmit);
  }

  async function onSetupSubmit(ev) {
    ev.preventDefault();
    const fd = new FormData(ev.target);
    const msg = $("#gate-msg");
    if (fd.get("password") !== fd.get("password2")) {
      msg.className = "msg err";
      msg.textContent = "Passwords do not match.";
      return;
    }
    try {
      const body = {
        email: fd.get("email"),
        password: fd.get("password"),
        enable_totp: !!fd.get("enable_totp"),
      };
      const res = await api("/auth/setup", { method: "POST", body: JSON.stringify(body) });
      if (res.token) saveSession(res.token);
      if (res.totp) {
        state.totpEnroll = res.totp;
        state.gate = "totp_enroll";
        renderGate();
        return;
      }
      await enterDashboard();
    } catch (e) {
      msg.className = "msg err";
      msg.textContent = e.message;
    }
  }

  async function onLoginSubmit(ev) {
    ev.preventDefault();
    const fd = new FormData(ev.target);
    const msg = $("#gate-msg");
    try {
      const res = await api("/auth/login", {
        method: "POST",
        body: JSON.stringify({ email: fd.get("email"), password: fd.get("password") }),
      });
      if (res.totp_required) {
        state.pendingToken = res.pending_token;
        state.pendingEmail = res.email || fd.get("email");
        state.gate = "totp";
        renderGate();
        return;
      }
      if (res.token) saveSession(res.token);
      await enterDashboard();
    } catch (e) {
      msg.className = "msg err";
      msg.textContent = e.message;
    }
  }

  async function onTotpSubmit(ev) {
    ev.preventDefault();
    const fd = new FormData(ev.target);
    const msg = $("#gate-msg");
    try {
      const res = await api("/auth/login/totp", {
        method: "POST",
        body: JSON.stringify({ pending_token: state.pendingToken, code: fd.get("code") }),
      });
      if (res.token) saveSession(res.token);
      state.pendingToken = null;
      await enterDashboard();
    } catch (e) {
      msg.className = "msg err";
      msg.textContent = e.message;
    }
  }

  async function onEnrollSubmit(ev) {
    ev.preventDefault();
    const fd = new FormData(ev.target);
    const msg = $("#gate-msg");
    try {
      await api("/auth/totp/enable", {
        method: "POST",
        body: JSON.stringify({ code: fd.get("code") }),
      });
      state.totpEnroll = null;
      await enterDashboard();
    } catch (e) {
      msg.className = "msg err";
      msg.textContent = e.message;
    }
  }

  async function enterDashboard() {
    state.gate = null;
    showApp(true);
    closeNav();
    parseRoute();
    try {
      await loadHealth();
      await loadAuthStatus();
      await loadMetrics();
      await loadProcesses();
      await loadConfig();
      startPoll();
      if (state.route === "logs") await loadLogs();
      if (state.route === "audit") await loadAudit();
      if (state.route === "security") await loadAuthStatus();
      render();
    } catch (e) {
      setHealthPill(false, "offline");
      $("#view").innerHTML = `<div class="msg err">Failed to load dashboard: ${esc(e.message)}</div>`;
    }
  }

  async function doLogout() {
    try { await api("/auth/logout", { method: "POST", body: "{}" }); } catch { /* ignore */ }
    saveSession("");
    state.gate = "login";
    if (state.pollTimer) clearInterval(state.pollTimer);
    renderGate();
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
    ["#health-pill", "#health-pill-mobile"].forEach(sel => {
      const el = $(sel);
      if (!el) return;
      el.textContent = text;
      el.className = "pill " + (ok ? "ok" : "bad") + (sel.includes("mobile") ? " mobile-only" : "");
    });
  }

  function setActiveNav(route) {
    $$(".nav-item").forEach(a => {
      a.classList.toggle("active", a.dataset.route === route);
    });
    $$(".bottom-nav-item[data-route]").forEach(a => {
      a.classList.toggle("active", a.dataset.route === route);
    });
    // "More" highlights when on secondary pages
    const more = $("#bottom-more");
    if (more) {
      const primary = ["overview", "metrics", "processes", "logs"];
      more.classList.toggle("active", !primary.includes(route));
    }
  }

  const titles = {
    overview: "Overview",
    metrics: "Performance",
    processes: "Processes",
    logs: "Logs",
    config: "Configuration",
    security: "Security",
    audit: "Audit Journal",
  };

  function render() {
    if (state.gate) {
      renderGate();
      return;
    }
    const view = $("#view");
    $("#page-title").textContent = titles[state.route] || "SMOS";
    setActiveNav(state.route);
    if (state.health) {
      const label = state.health.host_label || "smos-host";
      $("#host-label").textContent = label;
      $("#version").textContent = "v" + (state.health.version || "?");
      const topHost = $("#topbar-host");
      if (topHost) topHost.textContent = label;
    }
    if (state.auth?.email) {
      $("#user-email").textContent = state.auth.email;
    }
    try {
      switch (state.route) {
        case "metrics": view.innerHTML = renderMetrics(); break;
        case "processes": view.innerHTML = renderProcesses(); break;
        case "logs": view.innerHTML = renderLogs(); break;
        case "config": view.innerHTML = renderConfig(); break;
        case "security": view.innerHTML = renderSecurity(); break;
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
    const al = state.alerts;
    const alertBanner = al ? `
      <div class="card full ${al.any_breached ? "alert-breach" : "alert-ok"}">
        <h3>Alert status ${al.any_breached ? "· " + al.breach_count + " breach(es)" : "· all clear"}</h3>
        <p class="muted">Thresholds: CPU ≥ ${al.thresholds?.cpu_percent ?? "—"}% · Mem ≥ ${al.thresholds?.memory_percent ?? "—"}% · Disk ≥ ${al.thresholds?.disk_percent ?? "—"}%</p>
        <div class="chip-row" style="margin-top:0.5rem">
          <span class="chip ${al.cpu?.breached ? "danger" : ""}">CPU ${fmtPct(al.cpu?.current)} ${al.cpu?.breached ? "⚠" : "✓"}</span>
          <span class="chip ${al.memory?.breached ? "danger" : ""}">Mem ${fmtPct(al.memory?.current)} ${al.memory?.breached ? "⚠" : "✓"}</span>
          ${(al.disks || []).slice(0, 4).map(d =>
            `<span class="chip ${d.breached ? "danger" : ""}">Disk ${esc(d.scope || "?")} ${fmtPct(d.current)} ${d.breached ? "⚠" : "✓"}</span>`
          ).join("")}
        </div>
      </div>` : "";
    return `
      <div class="grid">
        ${alertBanner}
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
          <div class="table-wrap table-wrap-auto">
            <table>
              <tr><th>Hostname</th><td class="mono">${esc(m.hostname)}</td></tr>
              <tr><th>Label</th><td class="mono">${esc(h?.host_label || "—")}</td></tr>
              <tr><th>Host uptime</th><td class="mono">${fmtUptime(m.uptime_secs)}</td></tr>
              <tr><th>SMOS uptime</th><td class="mono">${fmtUptime(h?.uptime_secs)}</td></tr>
              <tr><th>Processes</th><td class="mono">${state.processes.length}</td></tr>
              <tr><th>Network ifaces</th><td class="mono">${(m.networks || []).length}</td></tr>
            </table>
          </div>
        </div>
        ${(m.networks && m.networks.length) ? `
        <div class="card wide">
          <h3>Network (top interfaces)</h3>
          <div class="table-wrap table-wrap-auto">
            <table>
              <thead><tr><th>Iface</th><th>RX</th><th>TX</th></tr></thead>
              <tbody>
                ${m.networks.slice(0, 6).map(n => `
                  <tr>
                    <td class="mono">${esc(n.name)}</td>
                    <td class="mono">${fmtBytes(n.bytes_received)}</td>
                    <td class="mono">${fmtBytes(n.bytes_transmitted)}</td>
                  </tr>`).join("")}
              </tbody>
            </table>
          </div>
        </div>` : ""}
        <div class="card">
          <h3>Quick links</h3>
          <div class="quick-links">
            <a class="btn secondary" href="#/processes">Manage processes</a>
            <a class="btn secondary" href="#/logs">Browse logs</a>
            <a class="btn secondary" href="#/audit">Audit journal</a>
            <a class="btn secondary" href="#/security">Security & 2FA</a>
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
      `<button type="button" class="chip ${hours === h ? "active" : ""}" data-hist-hours="${h}">${label}</button>`
    ).join("");
    const count = state.metricsHistory?.count ?? 0;
    const ret = state.metricsHistory?.retention_days
      ?? state.config?.history_retention_days
      ?? 30;
    return `
      <div class="card full">
        <h3>Stored history</h3>
        <p class="muted">Samples every ${state.config?.metrics_history_interval_secs || 60}s · kept <strong>${ret} days</strong></p>
        <div class="chip-row" role="group" aria-label="History range">${buttons}</div>
        <p class="muted" style="margin-top:0.55rem">${count} points · last ${hours}h</p>
        <div class="grid" style="margin-top:0.85rem">
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
          <div class="table-wrap table-wrap-auto">
            <table><thead><tr><th>Core</th><th>Usage</th><th></th></tr></thead><tbody>${cores || '<tr><td colspan="3" class="empty">No core data</td></tr>'}</tbody></table>
          </div>
        </div>
        <div class="card">
          <h3>Load average</h3>
          <div class="metric-sub mono">${m.load_avg ? `${m.load_avg.one.toFixed(2)} / ${m.load_avg.five.toFixed(2)} / ${m.load_avg.fifteen.toFixed(2)}` : "n/a on this OS"}</div>
        </div>
        <div class="card full">
          <h3>Disks</h3>
          <div class="table-wrap">
            <table>
              <thead><tr><th>Mount</th><th>FS</th><th>Used</th><th>%</th></tr></thead>
              <tbody>${disks || '<tr><td colspan="4" class="empty">No disks</td></tr>'}</tbody>
            </table>
          </div>
        </div>
        <div class="card full">
          <h3>Network interfaces</h3>
          <p class="muted">Cumulative counters from the host (sysinfo). RX/TX since interface up / boot.</p>
          <div class="table-wrap">
            <table>
              <thead><tr><th>Interface</th><th>RX bytes</th><th>TX bytes</th><th>RX pkts</th><th>TX pkts</th></tr></thead>
              <tbody>${(m.networks || []).map(n => `
                <tr>
                  <td class="mono">${esc(n.name)}</td>
                  <td class="mono">${fmtBytes(n.bytes_received)}</td>
                  <td class="mono">${fmtBytes(n.bytes_transmitted)}</td>
                  <td class="mono">${n.packets_received ?? 0}</td>
                  <td class="mono">${n.packets_transmitted ?? 0}</td>
                </tr>`).join("") || '<tr><td colspan="5" class="empty">No network interfaces</td></tr>'}
            </tbody>
            </table>
          </div>
        </div>
      </div>`;
  }

  function renderProcesses() {
    const f = state.processFilter || { name: "", sort: "cpu", order: "desc" };
    const list = (state.processes || []).slice(0, 150);
    const rows = list.map(p => `
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
    const cards = list.map(p => `
      <article class="proc-card">
        <div class="proc-card-top">
          <div>
            <div class="proc-name">${esc(p.name)}</div>
            <div class="proc-meta">
              <span>PID ${p.pid}</span>
              <span>CPU ${fmtPct(p.cpu_usage)}</span>
              <span>Mem ${fmtBytes(p.memory_bytes)}</span>
              <span>${esc(p.status)}</span>
            </div>
          </div>
        </div>
        <div class="proc-cmd">${esc((p.cmd || p.exe || "—").slice(0, 180))}</div>
        <div class="row-actions">
          <button class="btn secondary" data-act="terminate" data-pid="${p.pid}">Terminate</button>
          <button class="btn danger" data-act="kill" data-pid="${p.pid}">Kill</button>
        </div>
      </article>
    `).join("");
    const sortOpts = ["cpu", "memory", "name", "pid"].map(s =>
      `<option value="${s}" ${f.sort === s ? "selected" : ""}>${s}</option>`
    ).join("");
    return `
      <div class="grid">
        <div class="card full">
          <h3>Running processes (${state.processes.length})</h3>
          <p class="muted">Search and sort run on the server. Terminate is graceful; Kill forces exit. Self-process actions are blocked.</p>
          <form id="proc-filter-form" class="log-toolbar proc-filter" style="flex-wrap:wrap;gap:0.5rem;margin:0.75rem 0">
            <label class="muted">Name <input id="proc-name" name="name" type="search" placeholder="filter by name…" value="${esc(f.name || "")}" style="min-width:10rem" /></label>
            <label class="muted">Sort
              <select id="proc-sort" name="sort">${sortOpts}</select>
            </label>
            <label class="muted">Order
              <select id="proc-order" name="order">
                <option value="desc" ${f.order === "desc" ? "selected" : ""}>desc</option>
                <option value="asc" ${f.order === "asc" ? "selected" : ""}>asc</option>
              </select>
            </label>
            <button class="btn" type="submit">Apply</button>
            <button class="btn secondary" type="button" id="proc-clear">Clear</button>
          </form>
          <div class="proc-list">${cards || '<div class="empty">No processes match</div>'}</div>
          <div class="table-wrap proc-table-wrap" style="max-height:70vh;overflow:auto">
            <table>
              <thead><tr><th>PID</th><th>Name</th><th>CPU</th><th>Mem</th><th>Status</th><th>Actions</th></tr></thead>
              <tbody>${rows || '<tr><td colspan="6" class="empty">No processes match</td></tr>'}</tbody>
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
            <label>Alert CPU % ≥<input name="alert_cpu_percent" type="number" min="0" max="100" step="0.1" value="${c.alert_cpu_percent ?? 90}" /></label>
            <label>Alert memory % ≥<input name="alert_memory_percent" type="number" min="0" max="100" step="0.1" value="${c.alert_memory_percent ?? 90}" /></label>
            <label>Alert disk % ≥<input name="alert_disk_percent" type="number" min="0" max="100" step="0.1" value="${c.alert_disk_percent ?? 90}" /></label>
            <label class="full">Data dir<input value="${esc(c.data_dir)}" disabled /></label>
            <label class="full">Auth token set: <strong>${c.auth_token_set ? "yes" : "no"}</strong> (set via CLI/env SMOS_AUTH_TOKEN)</label>
            <div class="form-actions full">
              <button class="btn" type="submit">Save changes</button>
            </div>
          </form>
          <div id="config-msg"></div>
          ${state.alerts ? `
          <div class="muted" style="margin-top:0.75rem">
            Live: ${state.alerts.any_breached ? `<strong style="color:var(--danger)">${state.alerts.breach_count} threshold breach(es)</strong>` : "no breaches"}
            · <a href="#/overview">Overview</a> shows current status vs thresholds.
          </div>` : ""}
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

  function renderSecurity() {
    const a = state.auth || {};
    const totpOn = !!a.totp_enabled;
    return `
      <div class="grid">
        <div class="card wide">
          <h3>Operator account</h3>
          <table>
            <tr><th>Email</th><td class="mono">${esc(a.email || "—")}</td></tr>
            <tr><th>Session</th><td class="mono">${a.authenticated ? "active" : "none"}</td></tr>
            <tr><th>Offline 2FA (TOTP)</th><td class="mono">${totpOn ? "enabled" : "disabled"}</td></tr>
          </table>
          <p class="muted" style="margin-top:0.75rem">TOTP codes are generated on your phone offline (RFC 6238). No SMS or network is required after the secret is stored in your authenticator app.</p>
        </div>
        <div class="card">
          <h3>Two-factor authentication</h3>
          ${totpOn ? `
            <p class="muted">2FA is on. Disable requires password + current OTP.</p>
            <form id="totp-disable-form" class="form-grid" style="grid-template-columns:1fr">
              <label>Password<input name="password" type="password" required autocomplete="current-password" /></label>
              <label>Current OTP<input name="code" class="otp-input" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required /></label>
              <div class="form-actions"><button class="btn danger" type="submit">Disable 2FA</button></div>
            </form>
          ` : `
            <p class="muted">Optional offline OTP. Use Google Authenticator, Aegis, Authy, etc.</p>
            <button class="btn" id="totp-begin">Set up authenticator</button>
          `}
          <div id="security-msg" style="margin-top:0.75rem"></div>
        </div>
        ${state.totpEnroll ? `
        <div class="card full">
          <h3>Confirm enrollment</h3>
          <p class="muted">${esc(state.totpEnroll.note || "")}</p>
          <div class="secret-box">${esc(state.totpEnroll.secret || "")}</div>
          <form id="totp-enable-form" style="margin-top:0.75rem;display:grid;gap:0.75rem;max-width:320px">
            <label>6-digit code<input name="code" class="otp-input" inputmode="numeric" pattern="[0-9]{6}" maxlength="6" required /></label>
            <button class="btn" type="submit">Verify & enable</button>
          </form>
        </div>` : ""}
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
          <div class="table-wrap">
            <table>
              <thead><tr><th>Time</th><th>Action</th><th>Target</th><th>Result</th><th>Detail</th></tr></thead>
              <tbody>${rows || '<tr><td colspan="5" class="empty">No audit entries yet</td></tr>'}</tbody>
            </table>
          </div>
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

    const procForm = $("#proc-filter-form");
    if (procForm) {
      procForm.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        state.processFilter = {
          name: ($("#proc-name")?.value || "").trim(),
          sort: $("#proc-sort")?.value || "cpu",
          order: $("#proc-order")?.value || "desc",
        };
        try {
          await loadProcesses();
          render();
        } catch (e) {
          alert(e.message);
        }
      });
    }
    const procClear = $("#proc-clear");
    if (procClear) {
      procClear.addEventListener("click", async () => {
        state.processFilter = { name: "", sort: "cpu", order: "desc" };
        try {
          await loadProcesses();
          render();
        } catch (e) {
          alert(e.message);
        }
      });
    }

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
          alert_cpu_percent: Number(fd.get("alert_cpu_percent")),
          alert_memory_percent: Number(fd.get("alert_memory_percent")),
          alert_disk_percent: Number(fd.get("alert_disk_percent")),
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

    const totpBegin = $("#totp-begin");
    if (totpBegin) {
      totpBegin.addEventListener("click", async () => {
        const msg = $("#security-msg");
        try {
          state.totpEnroll = await api("/auth/totp/setup", { method: "POST", body: "{}" });
          render();
        } catch (e) {
          msg.className = "msg err";
          msg.textContent = e.message;
        }
      });
    }
    const totpEnable = $("#totp-enable-form");
    if (totpEnable) {
      totpEnable.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        const fd = new FormData(totpEnable);
        const msg = $("#security-msg");
        try {
          await api("/auth/totp/enable", {
            method: "POST",
            body: JSON.stringify({ code: fd.get("code") }),
          });
          state.totpEnroll = null;
          await loadAuthStatus();
          msg.className = "msg ok";
          msg.textContent = "Offline 2FA enabled.";
          render();
        } catch (e) {
          msg.className = "msg err";
          msg.textContent = e.message;
        }
      });
    }
    const totpDisable = $("#totp-disable-form");
    if (totpDisable) {
      totpDisable.addEventListener("submit", async (ev) => {
        ev.preventDefault();
        const fd = new FormData(totpDisable);
        const msg = $("#security-msg");
        try {
          await api("/auth/totp/disable", {
            method: "POST",
            body: JSON.stringify({ password: fd.get("password"), code: fd.get("code") }),
          });
          await loadAuthStatus();
          msg.className = "msg ok";
          msg.textContent = "2FA disabled.";
          render();
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

  async function loadAuthStatus() {
    state.auth = await api("/auth/status");
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

  async function loadAlerts() {
    state.alerts = await api("/alerts");
  }

  async function loadMetricsHistory() {
    const hours = state.historyRangeHours || 24;
    state.metricsHistory = await api(`/metrics/history?hours=${hours}&limit=500`);
  }

  async function loadHistoryStatus() {
    state.historyStatus = await api("/history");
  }

  async function loadProcesses() {
    const f = state.processFilter || { name: "", sort: "cpu", order: "desc" };
    const params = new URLSearchParams();
    if (f.name && f.name.trim()) params.set("name", f.name.trim());
    if (f.sort) params.set("sort", f.sort);
    if (f.order) params.set("order", f.order);
    params.set("limit", "200");
    const q = params.toString();
    state.processes = await api("/processes" + (q ? "?" + q : ""));
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
    if (state.gate) return;
    try {
      await loadHealth();
      await loadAuthStatus();
      await loadMetrics();
      if (state.route === "overview" || state.route === "config" || state.route === "metrics") {
        try { await loadAlerts(); } catch { /* optional */ }
      }
      if (state.route === "processes" || state.route === "overview") await loadProcesses();
      if (state.route === "metrics" || state.route === "overview") {
        try { await loadMetricsHistory(); } catch { /* empty history ok */ }
      }
      if (state.route === "logs") await loadLogs();
      if (state.route === "config") await loadConfig();
      if (state.route === "security") await loadAuthStatus();
      if (state.route === "audit") await loadAudit();
      render();
    } catch (e) {
      if (e.status === 401) {
        saveSession("");
        state.gate = "login";
        renderGate();
        return;
      }
      if (e.status === 403) {
        state.gate = "setup";
        renderGate();
        return;
      }
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
    if (state.gate) return;
    closeNav();
    parseRoute();
    refreshAll();
  }

  function startPoll() {
    if (state.pollTimer) clearInterval(state.pollTimer);
    const secs = state.config?.metrics_poll_secs || 2;
    state.pollTimer = setInterval(async () => {
      if (document.hidden || state.gate) return;
      try {
        await loadMetrics();
        if (state.route === "overview" || state.route === "metrics") render();
        if (state.route === "overview") {
          await loadHealth();
        }
      } catch { /* keep last good data */ }
    }, Math.max(1, secs) * 1000);
  }

  // shell chrome
  $("#refresh-btn")?.addEventListener("click", () => refreshAll());
  $("#logout-btn")?.addEventListener("click", () => {
    closeNav();
    doLogout();
  });
  $("#nav-open")?.addEventListener("click", () => toggleNav());
  $("#nav-close")?.addEventListener("click", () => closeNav());
  $("#nav-backdrop")?.addEventListener("click", () => closeNav());
  $("#bottom-more")?.addEventListener("click", () => openNav());
  $("#nav")?.addEventListener("click", (ev) => {
    if (ev.target.closest("a.nav-item")) closeNav();
  });
  window.addEventListener("keydown", (ev) => {
    if (ev.key === "Escape") closeNav();
  });
  window.addEventListener("hashchange", onRoute);
  window.addEventListener("resize", () => {
    if (window.innerWidth > 900) closeNav();
  });

  (async function init() {
    parseRoute();
    try {
      const status = await api("/auth/status");
      state.auth = status;
      if (status.setup_required) {
        state.gate = "setup";
        renderGate();
        return;
      }
      if (!status.authenticated) {
        // Have a stored session that may still be valid?
        if (sessionToken()) {
          try {
            const me = await api("/auth/me");
            if (me.authenticated) {
              await enterDashboard();
              return;
            }
          } catch {
            saveSession("");
          }
        }
        state.gate = "login";
        renderGate();
        return;
      }
      await enterDashboard();
    } catch (e) {
      $("#auth-gate").classList.remove("hidden");
      $("#auth-gate").innerHTML = `<div class="auth-card"><h1>SMOS</h1><p class="msg err">Cannot reach API: ${esc(e.message)}</p></div>`;
    }
  })();
})();
