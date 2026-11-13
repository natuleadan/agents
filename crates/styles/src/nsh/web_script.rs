/// Generates `/assets/app.js` driving browser interactions against `agents-server`.
#[must_use]
pub fn app_js() -> &'static str {
    r#"(function () {
  "use strict";
  const el = (id) => document.getElementById(id);
  const state = { token: "", sessionId: "", mode: "server", latency: [16, 21, 14, 19, 13] };

  function apiBase() {
    const val = (el("login-url-input").value || "").trim().replace(/\/+$/, "");
    return val || window.location.origin;
  }

  function showError(msg) {
    const box = el("login-error-banner");
    if (!msg) { box.classList.add("hidden"); box.textContent = ""; return; }
    box.textContent = msg;
    box.classList.remove("hidden");
  }

  function renderChart() {
    const svg = el("telemetry-svg");
    if (!svg) return;
    const pts = state.latency;
    const maxV = Math.max(10, ...pts);
    const poly = pts.map((v, i) => {
      const x = (i / Math.max(1, pts.length - 1)) * 380 + 10;
      const y = 88 - (v / maxV) * 72;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    }).join(" ");
    svg.innerHTML = `<polyline fill="none" stroke="var(--chart-1)" stroke-width="2.5" points="${poly}" />`;
  }

  function renderNodes(nodes) {
    const body = el("cluster-nodes-body");
    if (!body) return;
    if (!nodes || nodes.length === 0) {
      body.innerHTML = '<tr><td colspan="5" class="muted-cell">No private-VLAN Boxer workers registered yet.</td></tr>';
      return;
    }
    body.innerHTML = nodes.map((n) => {
      const esc = (s) => String(s).replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;" }[c]));
      const nodeId = n.nodeId || n.node_id || "";
      const vlanIp = n.vlanIp || n.vlan_ip || "";
      const rt = (n.runtime && typeof n.runtime === "object") ? (n.runtime.provider || n.runtime.kind || "") : (n.runtime || "");
      const iso = n.isolation || "";
      const active = n.activeBoxes ?? n.active_boxes ?? 0;
      const maxBoxes = n.maxConcurrentBoxes ?? n.max_boxes ?? 0;
      return `<tr><td>${esc(nodeId)}</td><td>${esc(vlanIp)}</td><td>${esc(rt)}/${esc(iso)}</td><td>${Number(active)}/${Number(maxBoxes)}</td><td>${esc(n.status)}</td></tr>`;
    }).join("");
  }

  async function refreshNodes() {
    if (!state.token) return;
    const t0 = performance.now();
    try {
      const resp = await fetch(`${apiBase()}/api/v1/nodes`, {
        headers: { Authorization: `Bearer ${state.token}` },
      });
      if (resp.ok) {
        const data = await resp.json();
        renderNodes(data.nodes || []);
      }
    } finally {
      state.latency.push(Math.max(1, Math.round(performance.now() - t0)));
      if (state.latency.length > 24) state.latency.shift();
      renderChart();
    }
  }

  function setAuthenticated(authed, email) {
    const badge = el("auth-state-badge");
    if (authed) {
      el("login-screen").classList.add("hidden");
      el("workspace-layout").classList.remove("hidden");
      badge.textContent = email || "Authenticated";
      renderChart();
      void refreshNodes();
    } else {
      el("workspace-layout").classList.add("hidden");
      el("login-screen").classList.remove("hidden");
      badge.textContent = "Signed Out";
    }
  }

  el("theme-toggle-btn").addEventListener("click", () => {
    const root = document.documentElement;
    root.dataset.theme = root.dataset.theme === "dark" ? "light" : "dark";
  });

  el("mode-server-btn").addEventListener("click", () => {
    state.mode = "server";
    el("mode-server-btn").classList.add("secondary");
    el("mode-server-btn").classList.remove("outline");
    el("mode-thirdparty-btn").classList.add("outline");
    el("mode-thirdparty-btn").classList.remove("secondary");
  });

  el("mode-thirdparty-btn").addEventListener("click", () => {
    state.mode = "thirdparty";
    el("mode-thirdparty-btn").classList.add("secondary");
    el("mode-thirdparty-btn").classList.remove("outline");
    el("mode-server-btn").classList.add("outline");
    el("mode-server-btn").classList.remove("secondary");
  });

  el("login-form").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    showError("");
    const email = el("login-email-input").value.trim();
    const password = el("login-pass-input").value;
    const btn = el("login-submit-btn");
    btn.disabled = true;
    btn.textContent = "Signing in...";
    try {
      const resp = await fetch(`${apiBase()}/api/v1/auth/login`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ email, password }),
      });
      if (!resp.ok) {
        showError(resp.status === 429 ? "Rate limit exceeded; retry shortly." : "Invalid credentials or server rejected login.");
        return;
      }
      const tokens = await resp.json();
      state.token = tokens.accessToken || tokens.access_token || "";
      setAuthenticated(true, email);
    } catch (err) {
      showError(`Connection failed: ${err && err.message ? err.message : "offline"}`);
    } finally {
      btn.disabled = false;
      btn.textContent = "Sign In & Connect";
    }
  });

  el("refresh-nodes-btn").addEventListener("click", () => { void refreshNodes(); });

  el("prompt-form").addEventListener("submit", async (ev) => {
    ev.preventDefault();
    if (!state.token) return;
    const prompt = el("composer-input").value.trim();
    if (!prompt) return;
    const out = el("stream-output");
    out.textContent = "Submitting session and task...";
    try {
      if (!state.sessionId) {
        const sResp = await fetch(`${apiBase()}/api/v1/sessions`, {
          method: "POST",
          headers: { "Content-Type": "application/json", Authorization: `Bearer ${state.token}` },
          body: JSON.stringify({ title: "Web Session", mode: "plan" }),
        });
        if (sResp.ok) {
          const s = await sResp.json();
          state.sessionId = s.id;
        }
      }
      if (!state.sessionId) { out.textContent = "Session creation failed."; return; }
      const tResp = await fetch(`${apiBase()}/api/v1/sessions/${encodeURIComponent(state.sessionId)}/tasks`, {
        method: "POST",
        headers: { "Content-Type": "application/json", Authorization: `Bearer ${state.token}` },
        body: JSON.stringify({ prompt, mode: "plan" }),
      });
      out.textContent = tResp.ok ? `Queued task: ${await tResp.text()}` : `Task error HTTP ${tResp.status}`;
    } catch (err) {
      out.textContent = `Task error: ${err && err.message ? err.message : "network"}`;
    }
  });

  el("palette-open-btn").addEventListener("click", () => {
    const dlg = el("command-palette-dialog");
    if (dlg && dlg.showModal) dlg.showModal();
  });
  el("palette-close-btn").addEventListener("click", () => el("command-palette-dialog").close());
  el("palette-refresh-btn").addEventListener("click", () => { el("command-palette-dialog").close(); void refreshNodes(); });
  el("palette-logout-btn").addEventListener("click", () => {
    state.token = "";
    state.sessionId = "";
    el("command-palette-dialog").close();
    setAuthenticated(false, "");
  });

  if (el("login-url-input")) el("login-url-input").value = window.location.origin;
  document.documentElement.dataset.appReady = "1";
})();
"#
}
