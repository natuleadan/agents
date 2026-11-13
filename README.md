# natuleadan-agents

Distributed, self-sustaining Rust agent harness combining an Ironic API orchestrator,
NATS JetStream swarm messaging, Valkey ephemeral state, PostgreSQL persistence,
modular Boxer sandbox execution, Traefik ephemeral previews, and multi-platform NSH clients.

## Crates

- **`agents-protocol`**: Shared protocol contracts, named codecs, JSON-RPC, and event streaming DTOs.
- **`agents-server`**: Central orchestrator daemon with Ironic NestJS-like modules, PostgreSQL, Valkey, NATS, MCP host, and autocron engine.
- **`agents-boxer`**: Sandbox daemon supervising `Runtime x Isolation` backends (`host`, `docker`, `openshell`), Traefik preview routes, and the native 2-level agent loop.
- **`agents-installer`**: VPS provisioning CLI coordinating external `.sh` hardening and cluster setup scripts.
- **`agents-client`**: Shared `client-core`, `client-tui`, and multi-platform `nsh` GUI (`web`, `ios`, `android`, `macos`, `windows`, `linux`) with `Server` and `Third-Party` modes.

## Development

```bash
# Install commitlint, Husky hooks, and workspace tooling
pnpm install

# Run full repository, formatting, clippy, test, and release-plan checks
pnpm validate
```

## Documentation & Governance

- [AGENTS.md](AGENTS.md) — technical architecture rules, branch model, and validation gates.
- [CONTRIBUTING.md](CONTRIBUTING.md) — commit policy and semantic release workflow.
- [SECURITY.md](SECURITY.md) — vulnerability disclosure and security scope.
- [NOTICE](NOTICE) — Apache-2.0 attribution notice.