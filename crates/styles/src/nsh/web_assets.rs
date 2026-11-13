use crate::tokens::Theme;

/// Returns the static Web App manifest (`manifest.webmanifest`) matching NSH `web.md`.
#[must_use]
pub fn web_manifest_json() -> &'static str {
    r##"{
  "name": "Agents Orchestrator",
  "short_name": "Agents",
  "start_url": "/",
  "display": "standalone",
  "background_color": "#0c0a09",
  "theme_color": "#b91c1c"
}
"##
}

/// Returns the root HTML shell for bare-IP HTTP and custom-domain HTTPS browser access.
#[must_use]
pub fn index_html() -> &'static str {
    r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Agents — NSH Control Plane</title>
  <link rel="manifest" href="/manifest.webmanifest" />
  <link rel="stylesheet" href="/assets/app.css" />
</head>
<body>
  <div id="app-root" data-testid="app-root" class="app-root">
    <header id="header-bar" data-testid="header-bar" class="header-bar">
      <div class="brand-group">
        <span class="brand-mark" data-testid="brand">agents</span>
        <span class="badge" data-testid="effective-profile-badge">server · docker + openshell</span>
        <span id="auth-state-badge" data-testid="auth-state-badge" class="badge muted">Signed Out</span>
      </div>
      <div class="header-actions">
        <div class="theme-switcher" role="group" aria-label="Theme preference">
          <select id="theme-select" data-testid="theme-select" class="select-theme" aria-label="Theme mode">
            <option value="system">System (Auto)</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>
          <button id="theme-toggle-btn" data-testid="theme-toggle-btn" type="button" class="btn outline" title="Cycle theme mode">Toggle Theme</button>
        </div>
        <button id="palette-open-btn" data-testid="palette-open-btn" type="button" class="btn ghost">Commands</button>
      </div>
    </header>

    <main id="main-shell" data-testid="main-shell" class="main-shell">
      <section id="login-screen" data-testid="login-screen" class="card login-card" aria-label="Sign In">
        <h1 class="card-title">Sign In to Agents Orchestrator</h1>
        <p class="card-desc">Connect in Server Mode or Third-Party Sandbox Mode.</p>
        <div class="mode-tabs" role="tablist" aria-label="Connection Mode">
          <button id="mode-server-btn" data-testid="mode-server-btn" type="button" class="btn secondary active" role="tab" aria-selected="true">Server Mode</button>
          <button id="mode-thirdparty-btn" data-testid="mode-thirdparty-btn" type="button" class="btn outline" role="tab" aria-selected="false">Third-Party</button>
        </div>
        <form id="login-form" class="form-stack">
          <label for="login-url-input" class="field-label">Control Plane URL</label>
          <input id="login-url-input" data-testid="login-url-input" class="input" type="url" value="" placeholder="http://127.0.0.1:8080" autocomplete="url" />
          <label for="login-email-input" class="field-label">Operator Email</label>
          <input id="login-email-input" data-testid="login-email-input" class="input" type="email" placeholder="operator@domain.com" autocomplete="username" required />
          <label for="login-pass-input" class="field-label">Password</label>
          <input id="login-pass-input" data-testid="login-pass-input" class="input" type="password" placeholder="Password" autocomplete="current-password" required />
          <button id="login-submit-btn" data-testid="login-submit-btn" type="submit" class="btn primary">Sign In &amp; Connect</button>
        </form>
        <div id="login-error-banner" data-testid="login-error-banner" class="alert hidden" role="alert"></div>
      </section>

      <div id="workspace-layout" data-testid="workspace-layout" class="workspace-layout hidden">
        <nav id="nav-region" data-testid="nav-region" class="sidebar" aria-label="Primary Navigation">
          <button data-testid="nav-workspace" data-view="workspace" type="button" class="btn secondary nav-btn active">Workspace</button>
          <button data-testid="nav-swarm" data-view="swarm" type="button" class="btn ghost nav-btn">Cluster &amp; Swarm</button>
          <button data-testid="nav-preview" data-view="preview" type="button" class="btn ghost nav-btn">Live Preview</button>
          <button data-testid="nav-updates" data-view="updates" type="button" class="btn ghost nav-btn">App Updates</button>
        </nav>

        <section id="workspace-panel" data-testid="workspace-panel" class="content-col">
          <div class="card">
            <h2 class="card-title">Agent Prompt &amp; Stream</h2>
            <form id="prompt-form" class="form-stack">
              <textarea id="composer-input" data-testid="composer-input" class="textarea" rows="3" placeholder="Ask agents to build, verify, or deploy..."></textarea>
              <div class="row-actions">
                <button id="run-prompt-btn" data-testid="run-prompt-btn" type="submit" class="btn primary">Run Task</button>
                <button id="refresh-nodes-btn" data-testid="refresh-nodes-btn" type="button" class="btn outline">Refresh VLAN Nodes</button>
              </div>
            </form>
            <pre id="stream-output" data-testid="stream-output" class="stream-box">Ready.</pre>
          </div>

          <div class="card">
            <h2 class="card-title">Private-VLAN Boxer Nodes</h2>
            <div class="table-wrap">
              <table id="cluster-nodes-table" data-testid="cluster-nodes-table" class="data-table">
                <thead>
                  <tr><th>Node ID</th><th>VLAN IP</th><th>Runtime</th><th>Load</th><th>Status</th></tr>
                </thead>
                <tbody id="cluster-nodes-body">
                  <tr><td colspan="5" class="muted-cell">Loading active nodes...</td></tr>
                </tbody>
              </table>
            </div>
          </div>

          <div class="card" id="telemetry-latency-chart" data-testid="telemetry-latency-chart">
            <h2 class="card-title">Stream Latency &amp; Token Telemetry</h2>
            <svg id="telemetry-svg" viewBox="0 0 400 100" class="chart-svg" role="img" aria-label="Telemetry chart"></svg>
          </div>
        </section>

        <aside id="preview-panel" data-testid="preview-panel" class="card preview-col">
          <h2 class="card-title">Traefik Live Preview</h2>
          <p id="preview-status" class="card-desc">No active preview route selected.</p>
          <iframe id="traefik-webview" data-testid="traefik-webview" title="Preview" class="preview-frame" src="about:blank" sandbox="allow-scripts allow-forms"></iframe>
        </aside>
      </div>
    </main>

    <dialog id="command-palette-dialog" data-testid="command-palette-dialog" class="modal card">
      <h2 class="card-title">Command Palette</h2>
      <p class="card-desc">Quick actions across server, swarm, and updates.</p>
      <div class="row-actions">
        <button id="palette-refresh-btn" type="button" class="btn secondary">Sync Cluster Nodes</button>
        <button id="palette-logout-btn" type="button" class="btn destructive">Sign Out</button>
        <button id="palette-close-btn" data-testid="palette-close-btn" type="button" class="btn outline">Close</button>
      </div>
    </dialog>
  </div>
  <script src="/assets/app.js" defer></script>
</body>
</html>
"#
}

/// Generates `/assets/app.css` combining design-token variables from [`Theme`] with responsive rules.
#[must_use]
pub fn app_css() -> String {
    let vars = Theme::default().to_css_stylesheet();
    format!(
        "{vars}\n{}",
        r#"*, *::before, *::after { box-sizing: border-box; }
html, body { margin: 0; padding: 0; background: var(--background); color: var(--foreground); font-family: var(--font-sans); min-height: 100vh; }
.hidden { display: none !important; }
.app-root { display: flex; flex-direction: column; min-height: 100vh; max-width: 1440px; margin: 0 auto; padding: 16px; gap: 16px; }
.header-bar { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 12px; padding: 12px 16px; background: var(--card); border: 1px solid var(--border); border-radius: var(--radius); }
.brand-group, .header-actions, .row-actions, .mode-tabs, .theme-switcher { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; }
.select-theme { background: var(--secondary); color: var(--secondary-foreground); border: 1px solid var(--border); border-radius: var(--radius); padding: 6px 10px; font-family: inherit; font-size: 0.84rem; cursor: pointer; }
.select-theme:focus { outline: 2px solid var(--ring); outline-offset: 1px; }
.brand-mark { font-weight: 700; font-size: 1.05rem; letter-spacing: 0.02em; }
.badge { display: inline-flex; align-items: center; padding: 3px 10px; border-radius: 999px; border: 1px solid var(--border); background: var(--secondary); color: var(--secondary-foreground); font-size: 0.78rem; }
.badge.muted { color: var(--muted-foreground); }
.main-shell { display: flex; flex-direction: column; gap: 16px; flex: 1; min-width: 0; }
.login-card { width: 100%; max-width: 460px; margin: 24px auto; }
.workspace-layout { display: grid; grid-template-columns: 220px minmax(0, 1fr) 340px; gap: 16px; align-items: start; min-width: 0; }
.sidebar { display: flex; flex-direction: column; gap: 6px; padding: 12px; background: var(--card); border: 1px solid var(--border); border-radius: var(--radius); }
.content-col { display: flex; flex-direction: column; gap: 16px; min-width: 0; }
.preview-col { display: flex; flex-direction: column; gap: 10px; min-width: 0; }
.card { background: var(--card); color: var(--card-foreground); border: 1px solid var(--border); border-radius: var(--radius); padding: 16px; min-width: 0; }
.card-title { margin: 0 0 6px; font-size: 1.02rem; }
.card-desc { margin: 0 0 12px; color: var(--muted-foreground); font-size: 0.88rem; }
.form-stack { display: flex; flex-direction: column; gap: 10px; }
.field-label { font-size: 0.84rem; color: var(--muted-foreground); }
.input, .textarea { width: 100%; padding: 9px 11px; border-radius: var(--radius); border: 1px solid var(--border); background: var(--background); color: var(--foreground); font-family: inherit; font-size: 0.92rem; }
.input:focus, .textarea:focus { outline: 2px solid var(--ring); outline-offset: 1px; }
.btn { display: inline-flex; align-items: center; justify-content: center; min-height: 36px; padding: 6px 14px; border-radius: var(--radius); border: 1px solid transparent; font-family: inherit; font-size: 0.88rem; cursor: pointer; background: var(--secondary); color: var(--secondary-foreground); }
.btn.primary { background: var(--primary); color: var(--primary-foreground); }
.btn.outline { background: transparent; color: var(--foreground); border-color: var(--border); }
.btn.ghost { background: transparent; color: var(--foreground); }
.btn.destructive { background: var(--destructive); color: var(--primary-foreground); }
.nav-btn { justify-content: flex-start; width: 100%; }
.alert { margin-top: 12px; padding: 10px 12px; border-radius: var(--radius); background: var(--destructive); color: var(--primary-foreground); font-size: 0.88rem; }
.stream-box { margin: 12px 0 0; padding: 10px; border-radius: var(--radius); background: var(--background); border: 1px solid var(--border); font-family: var(--font-mono); font-size: 0.84rem; white-space: pre-wrap; word-break: break-word; max-height: 180px; overflow: auto; }
.table-wrap { width: 100%; overflow-x: auto; }
.data-table { width: 100%; border-collapse: collapse; font-size: 0.86rem; }
.data-table th, .data-table td { text-align: left; padding: 8px 6px; border-bottom: 1px solid var(--border); }
.muted-cell { color: var(--muted-foreground); }
.chart-svg { width: 100%; height: 100px; display: block; }
.preview-frame { width: 100%; min-height: 240px; border: 1px solid var(--border); border-radius: var(--radius); background: var(--background); }
.modal { max-width: 440px; width: calc(100% - 32px); }
@media (max-width: 1024px) {
  .workspace-layout { grid-template-columns: 180px minmax(0, 1fr); }
  .preview-col { grid-column: 1 / -1; }
}
@media (max-width: 640px) {
  .app-root { padding: 10px; gap: 10px; }
  .workspace-layout { grid-template-columns: minmax(0, 1fr); }
  .sidebar { flex-direction: row; flex-wrap: wrap; }
  .nav-btn { width: auto; flex: 1 1 42%; min-height: 44px; justify-content: center; }
  .btn { min-height: 44px; }
}
"#
    )
}
