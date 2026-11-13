import assert from "node:assert/strict";
import { createServer } from "node:http";
import test from "node:test";
import { CloudApiClient, redactToken, runE2eCloud, withDropGuard } from "./e2e-cloud.ts";

function startMockCloudApi() {
  const servers = new Map();
  let seq = 100;

  const server = createServer((req, res) => {
    const auth = req.headers.authorization;
    if (auth !== "Bearer test-secret-token") {
      res.writeHead(401, { "content-type": "application/json" });
      res.end(JSON.stringify({ detail: "Unauthorized" }));
      return;
    }
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const url = req.url ?? "/";
      if (req.method === "GET" && url === "/projects/") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(
          JSON.stringify([
            {
              project: { id: 4601, name: "nla-main" },
              networks: [{ id: 187, location_name: "us-mia-1", ip_range: "10.0.0.0", prefix: 8 }],
            },
          ]),
        );
        return;
      }
      if (req.method === "GET" && url === "/sshkey/user/sshkeys") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ sshkeys: [{ id: 534, name: "ci-ed25519" }] }));
        return;
      }
      if (req.method === "GET" && url === "/vps/") {
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify([...servers.values()]));
        return;
      }
      if (req.method === "POST" && url.startsWith("/vps/create/")) {
        seq += 1;
        const body = JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
        const id = String(seq);
        const rec = {
          id,
          name: body.name,
          ipv4: body.ipv4 === false ? "" : `203.0.113.${seq - 90}`,
          private_ip: `10.0.1.${seq - 90}`,
        };
        servers.set(id, rec);
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ vps: rec }));
        return;
      }
      if (req.method === "POST" && url.startsWith("/vps/destroy/")) {
        const id = url.split("/").pop();
        servers.delete(id);
        res.writeHead(200, { "content-type": "application/json" });
        res.end(JSON.stringify({ destroyed: true }));
        return;
      }
      res.writeHead(404);
      res.end("not found");
    });
  });

  return { server, servers };
}

test("DropGuard cleans up all provisioned VPS nodes even when a step throws", async () => {
  const { server, servers } = startMockCloudApi();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address();
  const base = `http://127.0.0.1:${addr.port}`;
  const client = new CloudApiClient(base, "test-secret-token");
  const created = [];

  try {
    await assert.rejects(
      withDropGuard(created, client, false, async () => {
        const mono = await client.createVps("4601", { name: "agents-e2e-fail", ipv4: true });
        created.push(mono);
        assert.equal(servers.size, 1);
        throw new Error("simulated provision failure");
      }),
      /simulated provision failure/,
    );
    assert.equal(servers.size, 0);
    assert.equal((await client.listVps()).length, 0);
  } finally {
    server.close();
  }
});

test("dry-run cloud preflight logs no secrets and leaves zero instances", async () => {
  const { server, servers } = startMockCloudApi();
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const addr = server.address();
  const logs = [];
  const token = "test-secret-token";

  try {
    await runE2eCloud(
      {
        apiBase: `http://127.0.0.1:${addr.port}`,
        apiKey: token,
        projectId: "",
        vlanId: "",
        region: "us-mia-1",
        plan: "gp.nano",
        template: "debian-12",
        sshKeyId: "",
        sshKeyPath: ".local/id_ed25519",
        mode: "all",
        dryRun: true,
        keepDemo: false,
      },
      (line) => logs.push(line),
    );
    assert.equal(servers.size, 0);
    assert.ok(logs.some((l) => l.includes("[PASS] Dry-run verification completed")));
    assert.equal(redactToken(`Bearer ${token}`, token), "Bearer [REDACTED]");
  } finally {
    server.close();
  }
});
