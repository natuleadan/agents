#!/usr/bin/env node
// Ephemeral Cloud E2E harness for Mode A (Monolithic) and Mode B (Distributed VLAN) topologies.
// Reads credentials blindly from `.local/.env` or process.env; never prints secret values.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

export interface CloudConfig {
  apiBase: string;
  apiKey: string;
  projectId: string;
  vlanId: string;
  region: string;
  plan: string;
  template: string;
  sshKeyId: string;
  sshKeyPath: string;
  mode: "monolithic" | "distributed" | "all";
  dryRun: boolean;
  keepDemo: boolean;
}

export interface ProvisionedVps {
  id: string;
  name: string;
  publicIpv4: string;
  vlanIp: string;
  role: "monolithic" | "server" | "boxer";
}

export function parseEnvFile(path: string): Record<string, string> {
  if (!existsSync(path)) return {};
  const out: Record<string, string> = {};
  for (const rawLine of readFileSync(path, "utf8").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const eq = line.indexOf("=");
    if (eq <= 0) continue;
    const key = line.slice(0, eq).trim();
    let val = line.slice(eq + 1).trim();
    if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
      val = val.slice(1, -1);
    }
    if (val.length > 0) out[key] = val;
  }
  return out;
}

export function redactToken(text: string, secret: string): string {
  if (!secret) return text;
  return text.split(secret).join("[REDACTED]");
}

export function loadCloudConfig(argv: string[], env = process.env): CloudConfig {
  const localEnv = parseEnvFile(join(ROOT, ".local/.env"));
  const get = (keys: string[], fallback = ""): string => {
    for (const k of keys) {
      if (env[k] && env[k].trim()) return env[k].trim();
      if (localEnv[k] && localEnv[k].trim()) return localEnv[k].trim();
    }
    return fallback;
  };

  let mode: CloudConfig["mode"] = "all";
  let dryRun = true;
  let keepDemo = false;

  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--mode" && argv[i + 1]) {
      const m = argv[i + 1];
      if (m === "monolithic" || m === "distributed" || m === "all") mode = m;
      else throw new Error(`Unsupported --mode '${m}'`);
      i += 1;
    } else if (arg === "--dry-run") {
      dryRun = true;
    } else if (arg === "--live") {
      dryRun = false;
    } else if (arg === "--keep-demo") {
      keepDemo = true;
    } else if (arg === "-h" || arg === "--help") {
      console.log("Usage: node tools/e2e-cloud.ts [--mode monolithic|distributed|all] [--dry-run|--live] [--keep-demo]");
      process.exit(0);
    }
  }

  return {
    apiBase: get(["VPS_PROVIDER_API_BASE"]).replace(/\/+$/, ""),
    apiKey: get(["VPS_PROVIDER_API_KEY"]),
    projectId: get(["VPS_PROVIDER_PROJECT_ID"]),
    vlanId: get(["VPS_PROVIDER_VLAN_ID"]),
    region: get(["VPS_PROVIDER_REGION"], "us-mia-1"),
    plan: get(["VPS_PROVIDER_PLAN"], "gp.nano"),
    template: get(["VPS_PROVIDER_TEMPLATE"], "debian-12"),
    sshKeyId: get(["VPS_PROVIDER_SSH_KEY_ID"]),
    sshKeyPath: resolve(ROOT, get(["VPS_PROVIDER_SSH_KEY_PATH"], ".local/id_ed25519")),
    mode,
    dryRun,
    keepDemo,
  };
}

export class CloudApiClient {
  private readonly base: string;
  private readonly token: string;

  constructor(base: string, token: string) {
    this.base = base.replace(/\/+$/, "");
    this.token = token;
  }

  private async request(method: string, path: string, body?: unknown): Promise<unknown> {
    if (!this.base) throw new Error("VPS_PROVIDER_API_BASE is not configured");
    if (!this.token) throw new Error("VPS_PROVIDER_API_KEY is not configured");
    const res = await fetch(`${this.base}${path}`, {
      method,
      headers: {
        Authorization: `Bearer ${this.token}`,
        "Content-Type": "application/json",
        Accept: "application/json",
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await res.text();
    if (!res.ok) {
      throw new Error(redactToken(`HTTP ${res.status} on ${method} ${path}: ${text.slice(0, 300)}`, this.token));
    }
    return text ? JSON.parse(text) : null;
  }

  async listProjects(): Promise<Array<{ project: { id: number; name: string }; networks?: Array<{ id: number; location_name: string; ip_range: string; prefix: number }> }>> {
    return (await this.request("GET", "/projects/")) as Array<{
      project: { id: number; name: string };
      networks?: Array<{ id: number; location_name: string; ip_range: string; prefix: number }>;
    }>;
  }

  async listVps(): Promise<Array<{ id: string | number; name?: string; label?: string }>> {
    return (await this.request("GET", "/vps/")) as Array<{ id: string | number; name?: string; label?: string }>;
  }

  async listSshKeys(): Promise<Array<{ id: number; name: string; fingerprint?: string }>> {
    const res = (await this.request("GET", "/sshkey/user/sshkeys")) as { sshkeys?: Array<{ id: number; name: string; fingerprint?: string }> };
    return res.sshkeys ?? [];
  }

  async createVps(projectId: string, payload: Record<string, unknown>): Promise<ProvisionedVps> {
    const res = (await this.request("POST", `/vps/create/${encodeURIComponent(projectId)}`, payload)) as Record<string, unknown>;
    const vps = (res.vps as Record<string, unknown> | undefined) ?? res;
    return {
      id: String(vps.id ?? res.id ?? ""),
      name: String(vps.name ?? payload.name ?? ""),
      publicIpv4: String(vps.ipv4 ?? vps.public_ip ?? ""),
      vlanIp: String(vps.private_ip ?? vps.vlan_ip ?? ""),
      role: (payload.ipv4 === false ? "boxer" : payload.label === "server" ? "server" : "monolithic"),
    };
  }

  async destroyVps(vpsId: string): Promise<void> {
    await this.request("POST", `/vps/destroy/${encodeURIComponent(vpsId)}`);
  }
}

export async function withDropGuard<T>(
  created: ProvisionedVps[],
  client: CloudApiClient,
  keepDemo: boolean,
  fn: () => Promise<T>,
): Promise<T> {
  const cleanup = async () => {
    if (keepDemo) return;
    for (const vps of [...created].reverse()) {
      if (!vps.id) continue;
      try {
        await client.destroyVps(vps.id);
      } catch (err) {
        console.error(`[DROP-GUARD] Failed destroying ${vps.id}: ${String(err)}`);
      }
    }
  };

  try {
    return await fn();
  } finally {
    await cleanup();
  }
}

export function detectDirectSshBlocked(host = "github.com"): boolean {
  const probe = spawnSync(
    "ssh",
    ["-T", "-o", "ConnectTimeout=4", "-o", "BatchMode=yes", "-o", "StrictHostKeyChecking=no", `git@${host}`],
    { encoding: "utf8" },
  );
  const stderr = probe.stderr ?? "";
  return stderr.includes("HTTP/1.1 400 Bad Request") || stderr.includes("kex_exchange_identification");
}

export function buildTopologyPayloads(cfg: CloudConfig, runId: string) {
  const sshIds = cfg.sshKeyId ? [Number(cfg.sshKeyId)] : [];
  const monolithic = {
    name: `agents-e2e-mono-${runId}`,
    label: "monolithic",
    plan_name: cfg.plan,
    location_name: cfg.region,
    template_name: cfg.template,
    network_id: cfg.vlanId ? Number(cfg.vlanId) : null,
    ssh_key_ids: sshIds,
    ipv4: true,
    ipv6: true,
    enable_backups: false,
  };
  const distServer = {
    name: `agents-e2e-srv-${runId}`,
    label: "server",
    plan_name: cfg.plan,
    location_name: cfg.region,
    template_name: cfg.template,
    network_id: Number(cfg.vlanId),
    ssh_key_ids: sshIds,
    ipv4: true,
    ipv6: true,
    enable_backups: false,
  };
  const distBoxer = {
    name: `agents-e2e-box-${runId}`,
    label: "boxer",
    plan_name: cfg.plan,
    location_name: cfg.region,
    template_name: cfg.template,
    network_id: Number(cfg.vlanId),
    ssh_key_ids: sshIds,
    ipv4: false,
    ipv6: false,
    enable_backups: false,
  };
  return { monolithic, distServer, distBoxer };
}

export async function runE2eCloud(cfg: CloudConfig, logger: (line: string) => void = console.log): Promise<void> {
  const client = new CloudApiClient(cfg.apiBase, cfg.apiKey);
  const projects = await client.listProjects();
  if (projects.length === 0) throw new Error("No cloud projects found for provided credentials");
  const project = cfg.projectId
    ? projects.find((p) => String(p.project.id) === cfg.projectId) ?? projects[0]
    : projects[0];
  const resolvedProjectId = cfg.projectId || String(project.project.id);
  const resolvedVlan = cfg.vlanId || String(project.networks?.find((n) => n.location_name === cfg.region)?.id ?? "");
  const sshKeys = await client.listSshKeys();
  const resolvedSshKeyId = cfg.sshKeyId || (sshKeys[0] ? String(sshKeys[0].id) : "");

  const resolvedCfg: CloudConfig = {
    ...cfg,
    projectId: resolvedProjectId,
    vlanId: resolvedVlan,
    sshKeyId: resolvedSshKeyId,
  };

  const beforeVps = await client.listVps();
  const runId = Date.now().toString(36);
  const payloads = buildTopologyPayloads(resolvedCfg, runId);

  logger(
    `[INFO] Preflight ok: project=${resolvedProjectId} region=${resolvedCfg.region} plan=${resolvedCfg.plan} vlan=${resolvedVlan || "none"} existing_vps=${beforeVps.length}`,
  );

  if (cfg.dryRun) {
    logger(`[DRY-RUN] Mode A payload: ${payloads.monolithic.name} (${payloads.monolithic.plan_name})`);
    logger(`[DRY-RUN] Mode B payloads: ${payloads.distServer.name} + ${payloads.distBoxer.name} (ipv4=false, vlan=${resolvedVlan})`);
    logger("[PASS] Dry-run verification completed with zero cloud spend.");
    return;
  }

  if (detectDirectSshBlocked()) {
    throw new Error(
      "Direct outbound SSH (TCP port 22) is blocked by the sandbox HTTP proxy; aborting before creating billable VPS instances.",
    );
  }

  const created: ProvisionedVps[] = [];
  await withDropGuard(created, client, cfg.keepDemo, async () => {
    if (cfg.mode === "monolithic" || cfg.mode === "all") {
      const mono = await client.createVps(resolvedProjectId, payloads.monolithic);
      created.push(mono);
      logger(`[INFO] Created Mode A VPS id=${mono.id} ip=${mono.publicIpv4}`);
    }
    if (cfg.mode === "distributed" || cfg.mode === "all") {
      if (!resolvedVlan) throw new Error("Distributed Mode B requires a private VLAN network ID");
      const srv = await client.createVps(resolvedProjectId, payloads.distServer);
      created.push(srv);
      const box = await client.createVps(resolvedProjectId, payloads.distBoxer);
      created.push(box);
      logger(`[INFO] Created Mode B VPS pair server=${srv.id} (${srv.publicIpv4}) boxer=${box.id} (${box.vlanIp})`);
    }
  });

  if (!cfg.keepDemo) {
    const afterVps = await client.listVps();
    const leaked = afterVps.filter((v) => created.some((c) => c.id === String(v.id)));
    if (leaked.length > 0) {
      throw new Error(`Teardown verification failed; leaked VPS IDs: ${leaked.map((v) => v.id).join(", ")}`);
    }
    logger("[PASS] Zero orphan test VPS instances verified after DropGuard teardown.");
  }
}

const isEntrypoint = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (isEntrypoint) {
  const cfg = loadCloudConfig(process.argv.slice(2));
  runE2eCloud(cfg).catch((err) => {
    console.error(`[FAIL] ${redactToken(String(err), cfg.apiKey)}`);
    process.exit(1);
  });
}
