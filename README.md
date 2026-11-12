# natuleadan-agents

Distributed agent harness with modular Boxer sandbox execution, autocron workflows, and Ironic API orchestrator.

## Crates

- **`agents-protocol`**: Shared protocol definitions and event streaming types.
- **`agents-server`**: Central orchestrator with Ironic API, PostgreSQL, Valkey, NATS, and autocron engine.
- **`agents-boxer`**: Sandbox container supervisor and execution worker.
- **`agents-installer`**: VPS provisioning and environment self-configuration CLI.
- **`agents-client`**: Client interface for interacting with the orchestrator.

## Development

```bash
# Install commit linting & hooks
pnpm install

# Check Rust workspace
cargo check --workspace
cargo test --workspace
```

## Contributing & Releases

See [CONTRIBUTING.md](CONTRIBUTING.md) for commit rules and release workflows.