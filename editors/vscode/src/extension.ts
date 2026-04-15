import * as vscode from "vscode";
import * as http from "http";
import * as cp from "child_process";

let statusBar: vscode.StatusBarItem;
let pollTimer: ReturnType<typeof setInterval> | undefined;

interface ScopeonStats {
  today_cost_usd?: number;
  cache_hit_rate?: number;
  health_score?: number;
  context_fill_pct?: number;
}

function getPort(): number {
  return vscode.workspace.getConfiguration("scopeon").get<number>("port", 7771);
}

function getPollInterval(): number {
  return vscode.workspace.getConfiguration("scopeon").get<number>("pollIntervalMs", 30_000);
}

function fetchStats(): Promise<ScopeonStats | null> {
  return new Promise((resolve) => {
    const port = getPort();
    const req = http.get(
      { hostname: "127.0.0.1", port, path: "/api/v1/stats", timeout: 2000 },
      (res) => {
        let data = "";
        res.on("data", (chunk) => { data += chunk; });
        res.on("end", () => {
          try { resolve(JSON.parse(data) as ScopeonStats); }
          catch { resolve(null); }
        });
      }
    );
    req.on("error", () => resolve(null));
    req.on("timeout", () => { req.destroy(); resolve(null); });
  });
}

async function updateStatusBar() {
  const stats = await fetchStats();
  if (!stats) {
    statusBar.text = "$(beaker) Scopeon";
    statusBar.tooltip = "Scopeon is not running. Start with: scopeon start";
    statusBar.backgroundColor = undefined;
    return;
  }
  const cost = (stats.today_cost_usd ?? 0).toFixed(2);
  const cache = Math.round((stats.cache_hit_rate ?? 0) * 100);
  const health = stats.health_score ?? 0;
  const fill = Math.round((stats.context_fill_pct ?? 0) * 100);
  statusBar.text = `$(beaker) $${cost}  ${cache}% cache  ${fill}% ctx`;
  statusBar.tooltip = [
    `Scopeon AI Observability`,
    `Health: ${health}/100`,
    `Today cost: $${cost}`,
    `Cache hit: ${cache}%`,
    `Context fill: ${fill}%`,
    ``,
    `Click to open dashboard (http://127.0.0.1:${getPort()})`,
  ].join("\n");
  if (health < 50) {
    statusBar.backgroundColor = new vscode.ThemeColor("statusBarItem.warningBackground");
  } else {
    statusBar.backgroundColor = undefined;
  }
}

export function activate(context: vscode.ExtensionContext) {
  // Status bar item — shows cost + cache
  statusBar = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Right, 100);
  statusBar.command = "scopeon.openDashboard";
  statusBar.show();
  context.subscriptions.push(statusBar);

  // Commands
  context.subscriptions.push(
    vscode.commands.registerCommand("scopeon.openDashboard", () => {
      const port = getPort();
      vscode.env.openExternal(vscode.Uri.parse(`http://127.0.0.1:${port}`));
    })
  );
  context.subscriptions.push(
    vscode.commands.registerCommand("scopeon.openDigest", () => {
      const terminal = vscode.window.createTerminal({ name: "Scopeon Digest" });
      terminal.sendText("scopeon digest");
      terminal.show();
    })
  );

  // Start polling
  updateStatusBar();
  const interval = getPollInterval();
  pollTimer = setInterval(updateStatusBar, interval);
  context.subscriptions.push({ dispose: () => { if (pollTimer) clearInterval(pollTimer); } });

  // Re-poll immediately if configuration changes
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("scopeon")) {
        if (pollTimer) clearInterval(pollTimer);
        pollTimer = setInterval(updateStatusBar, getPollInterval());
        updateStatusBar();
      }
    })
  );
}

export function deactivate() {
  if (pollTimer) clearInterval(pollTimer);
}
