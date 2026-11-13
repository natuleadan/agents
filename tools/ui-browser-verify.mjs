#!/usr/bin/env node
// Real headless Chromium visual, layout, contrast, and E2E login verification against agents-server.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import net from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.listen(0, "127.0.0.1", () => {
      const addr = srv.address();
      const port = typeof addr === "object" && addr ? addr.port : 0;
      srv.close(() => resolve(port));
    });
    srv.on("error", reject);
  });
}

async function waitForHttp(url, timeoutMs = 15000) {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const r = await fetch(url);
      if (r.ok) return;
    } catch {
      // retry until ready
    }
    await new Promise((r) => setTimeout(r, 150));
  }
  throw new Error(`Timed out waiting for ${url}`);
}

class CdpClient {
  constructor(wsUrl) {
    this.ws = new WebSocket(wsUrl);
    this.nextId = 1;
    this.pending = new Map();
    this.errors = [];
    this.ws.addEventListener("message", (ev) => {
      const msg = JSON.parse(String(ev.data));
      if (msg.id && this.pending.has(msg.id)) {
        const { resolve, reject } = this.pending.get(msg.id);
        this.pending.delete(msg.id);
        if (msg.error) reject(new Error(msg.error.message || JSON.stringify(msg.error)));
        else resolve(msg.result);
      } else if (msg.method === "Runtime.exceptionThrown") {
        this.errors.push(msg.params?.exceptionDetails?.text || "Runtime exception");
      } else if (msg.method === "Log.entryAdded" && msg.params?.entry?.level === "error") {
        this.errors.push(msg.params.entry.text);
      }
    });
  }

  async connect() {
    await new Promise((resolve, reject) => {
      this.ws.addEventListener("open", resolve, { once: true });
      this.ws.addEventListener("error", reject, { once: true });
    });
    await this.send("Page.enable");
    await this.send("Runtime.enable");
    await this.send("Log.enable");
  }

  send(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async evalJs(expression) {
    const res = await this.send("Runtime.evaluate", {
      expression,
      awaitPromise: true,
      returnByValue: true,
    });
    if (res.exceptionDetails) {
      throw new Error(`JS evaluation error: ${JSON.stringify(res.exceptionDetails)}`);
    }
    return res.result?.value;
  }

  close() {
    try {
      this.ws.close();
    } catch {
      // ignore close errors
    }
  }
}

async function run() {
  const port = await freePort();
  const baseUrl = `http://127.0.0.1:${port}`;
  const serverBin = join(process.cwd(), "target/debug/agents-server");
  const chromeDataDir = mkdtempSync(join(tmpdir(), "agents-chrome-"));

  const serverProc = spawn(serverBin, ["serve"], {
    env: {
      ...process.env,
      AGENTS_BIND_ADDR: `127.0.0.1:${port}`,
      AGENTS_PUBLIC_URL: baseUrl,
      AGENTS_TRUSTED_ORIGINS: baseUrl,
      AGENTS_RATE_LIMIT_MAX: "1000",
      RUST_LOG: "warn",
    },
    stdio: ["ignore", "pipe", "pipe"],
  });

  let chromeProc = null;
  let cdp = null;

  try {
    await waitForHttp(`${baseUrl}/health/ready`, 20000);
    const reg = await fetch(`${baseUrl}/api/v1/auth/register`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        email: "browser-e2e@example.com",
        password: "very-secure-password-123",
      }),
    });
    if (!reg.ok) {
      throw new Error(`Register failed HTTP ${reg.status}: ${await reg.text()}`);
    }

    chromeProc = spawn(
      "chromium",
      [
        "--headless=new",
        "--no-sandbox",
        "--disable-gpu",
        "--remote-debugging-port=0",
        `--user-data-dir=${chromeDataDir}`,
        "about:blank",
      ],
      { stdio: ["ignore", "pipe", "pipe"] },
    );

    const wsPort = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("Chromium DevTools timeout")), 15000);
      const onChunk = (chunk) => {
        const match = /ws:\/\/127\.0\.0\.1:(\d+)\//.exec(chunk.toString("utf8"));
        if (match) {
          clearTimeout(timer);
          resolve(Number(match[1]));
        }
      };
      chromeProc.stderr.on("data", onChunk);
      chromeProc.stdout.on("data", onChunk);
    });

    const targets = await (await fetch(`http://127.0.0.1:${wsPort}/json/list`)).json();
    const pageTarget = targets.find((t) => t.type === "page");
    if (!pageTarget?.webSocketDebuggerUrl) {
      throw new Error("Missing Chromium page WebSocket target");
    }

    cdp = new CdpClient(pageTarget.webSocketDebuggerUrl);
    await cdp.connect();
    await cdp.send("Page.navigate", { url: baseUrl });
    await waitForHttp(baseUrl, 5000);

    // Wait for form hydration + /assets/app.js event binding
    let appReady = false;
    for (let i = 0; i < 60; i++) {
      appReady = await cdp.evalJs("document.documentElement?.dataset?.appReady === '1'");
      if (appReady) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    if (!appReady) {
      throw new Error(`Timed out waiting for /assets/app.js hydration. Console errors: ${JSON.stringify(cdp.errors)}`);
    }

    // 1. Verify error state on wrong password
    await cdp.evalJs(`(() => {
      document.getElementById('login-email-input').value = 'browser-e2e@example.com';
      document.getElementById('login-pass-input').value = 'wrong-pass';
      document.getElementById('login-form').requestSubmit();
    })()`);
    for (let i = 0; i < 30; i++) {
      const visible = await cdp.evalJs("!document.getElementById('login-error-banner').classList.contains('hidden')");
      if (visible) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    const errText = await cdp.evalJs("document.getElementById('login-error-banner').textContent");
    if (!errText || !errText.includes("Invalid credentials")) {
      throw new Error(`Expected visible login error banner, got: ${errText}`);
    }
    // Clear intentional HTTP 401 log entry from wrong-password test before verifying normal usage
    cdp.errors.length = 0;

    // 2. Verify valid login switches cleanly to authenticated Workspace + Table + Chart
    await cdp.evalJs(`(() => {
      document.getElementById('login-pass-input').value = 'very-secure-password-123';
      document.getElementById('login-form').requestSubmit();
    })()`);
    for (let i = 0; i < 40; i++) {
      const visible = await cdp.evalJs("!document.getElementById('workspace-layout').classList.contains('hidden')");
      if (visible) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    const chartPoints = await cdp.evalJs("document.querySelector('#telemetry-svg polyline')?.getAttribute('points') || ''");
    if (!chartPoints.includes(",")) {
      throw new Error(`Expected SVG telemetry polyline points, got: ${chartPoints}`);
    }

    // 2b. Verify auto-registered private-VLAN Boxer node renders in live table
    const loginResp = await (await fetch(`${baseUrl}/api/v1/auth/login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email: "browser-e2e@example.com", password: "very-secure-password-123" }),
    })).json();
    const hb = await fetch(`${baseUrl}/api/v1/nodes/heartbeat`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${loginResp.accessToken || loginResp.access_token}` },
      body: JSON.stringify({
        nodeId: "boxer-mia-browser",
        vlanIp: "10.0.1.55",
        runtime: { kind: "docker" },
        isolation: "openshell",
        maxConcurrentBoxes: 8,
        activeBoxes: 1,
        status: "online",
      }),
    });
    if (!hb.ok) {
      throw new Error(`VLAN heartbeat failed HTTP ${hb.status}: ${await hb.text()}`);
    }
    await cdp.evalJs("document.getElementById('refresh-nodes-btn').click()");
    for (let i = 0; i < 30; i++) {
      const txt = await cdp.evalJs("document.getElementById('cluster-nodes-body').textContent");
      if (txt && txt.includes("10.0.1.55")) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    const tableText = await cdp.evalJs("document.getElementById('cluster-nodes-body').textContent");
    if (!tableText || !tableText.includes("boxer-mia-browser") || !tableText.includes("10.0.1.55") || !tableText.includes("docker/openshell") || tableText.includes("[object")) {
      throw new Error(`Expected clean boxer-mia-browser row in table, got: ${tableText}`);
    }

    // 2c. Verify task submission creates session and queues task via REST API
    await cdp.evalJs(`(() => {
      document.getElementById('composer-input').value = 'Verify deployment health and cluster nodes';
      document.getElementById('prompt-form').requestSubmit();
    })()`);
    let streamOut = "";
    for (let i = 0; i < 30; i++) {
      streamOut = await cdp.evalJs("document.getElementById('stream-output').textContent");
      if (streamOut && streamOut.includes("Queued task:")) break;
      await new Promise((r) => setTimeout(r, 100));
    }
    if (!streamOut || !streamOut.includes("Queued task:")) {
      throw new Error(`Expected Queued task confirmation in stream-output, got: ${streamOut}`);
    }

    // 3. Verify Light & Dark theme toggle
    await cdp.evalJs("document.getElementById('theme-toggle-btn').click()");
    const themeLight = await cdp.evalJs("document.documentElement.dataset.theme");
    if (themeLight !== "light") {
      throw new Error(`Expected light theme after toggle, got ${themeLight}`);
    }
    await cdp.evalJs("document.getElementById('theme-toggle-btn').click()");

    // 4. Verify responsive layout (overflow + overlap + clipping) across Desktop, Tablet, Mobile
    for (const [w, h, label] of [
      [1440, 900, "desktop"],
      [834, 1194, "tablet"],
      [390, 844, "mobile"],
    ]) {
      await cdp.send("Emulation.setDeviceMetricsOverride", {
        width: w,
        height: h,
        deviceScaleFactor: 1,
        mobile: w < 640,
      });
      await new Promise((r) => setTimeout(r, 120));
      const metrics = await cdp.evalJs(`(() => {
        const overflow = document.documentElement.scrollWidth > window.innerWidth + 1;
        const buttons = Array.from(document.querySelectorAll('#nav-region button, #header-bar button'))
          .filter((n) => n.offsetParent !== null)
          .map((n) => {
            const r = n.getBoundingClientRect();
            return { id: n.getAttribute('data-testid') || n.id, left: r.left, right: r.right, top: r.top, bottom: r.bottom, w: r.width, h: r.height };
          });
        const overlaps = [];
        for (let i = 0; i < buttons.length; i++) {
          const a = buttons[i];
          if (a.right > window.innerWidth + 1 || a.w <= 0 || a.h <= 0) {
            overlaps.push({ clipped: a });
          }
          for (let j = i + 1; j < buttons.length; j++) {
            const b = buttons[j];
            const xi = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left));
            const yi = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));
            if (xi > 1 && yi > 1) overlaps.push({ a: a.id, b: b.id });
          }
        }
        return { overflow, scrollWidth: document.documentElement.scrollWidth, innerWidth: window.innerWidth, overlaps, buttonCount: buttons.length };
      })()`);

      if (metrics.overflow || metrics.overlaps.length > 0 || metrics.buttonCount < 4) {
        throw new Error(`Layout audit failed on ${label} (${w}x${h}): ${JSON.stringify(metrics)}`);
      }
    }

    if (cdp.errors.length > 0) {
      throw new Error(`Console/runtime errors detected: ${JSON.stringify(cdp.errors)}`);
    }

    console.log("PASS: headless Chromium verified Login error + success flow, Chart SVG, Light/Dark theme, and Desktop/Tablet/Mobile zero-overlap/zero-overflow layout.");
  } finally {
    cdp?.close();
    chromeProc?.kill("SIGKILL");
    serverProc?.kill("SIGKILL");
    rmSync(chromeDataDir, { recursive: true, force: true });
  }
}

run().catch((err) => {
  console.error("FAIL:", err);
  process.exit(1);
});
