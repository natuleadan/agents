# AGENTS.md

Technical instructions for agents and contributors working in `natuleadan/agents`.

## Overview

`natuleadan/agents` is a distributed, self-sustaining Rust agent harness
combining modular Ironic API orchestration, Boxer sandbox execution, NATS
JetStream swarm coordination, Valkey ephemeral state, PostgreSQL persistence,
Traefik ephemeral app previews, and multi-platform NSH clients.

## Repository Layout

| Path | Purpose |
|---|---|
| `crates/protocol` | Shared wire contracts, named codecs, JSON-RPC, HTTP DTOs, frozen fixtures |
| `crates/server` | Orchestrator daemon (`ironic` NestJS module tree, Postgres, Valkey, NATS, MCP, Autocron) |
| `crates/boxer` | Sandbox supervisor daemon, `Runtime x Isolation` backends, Traefik route publisher, native loop |
| `crates/installer` | VPS provisioning CLI orchestrating external `.sh` hardening scripts |
| `crates/client` | `client-core`, `client-tui`, and multi-platform `nsh` GUI (`Server` & `Third-Party` modes) |
| `deploy/scripts/` | Reusable POSIX/Bash VPS hardening and service bootstrap scripts (never hardcoded inside Rust) |
| `tools/` | Release planning (`release-plan.ts`) |
| `.github/` | CODEOWNERS, Dependabot, CI validation, and multi-platform release workflows |

## Development

Clone the repository and run `pnpm install` to install Husky hooks and
`commitlint`. Always use `pnpm` (never `npm` or `npx`).

```bash
pnpm install
pnpm validate
```

## Validation

`pnpm validate` is the single gate before pushing and enforces:

- `cargo fmt --all -- --check` passes cleanly.
- `cargo clippy --workspace --all-targets -- -D warnings` passes with zero warnings.
- `cargo test --workspace` passes completely.
- `node tools/release-plan.ts --check` matches `package.json` and `CHANGELOG.md`.

Repository hygiene rules (each `.rs`, `.ts`, and `.sh` file `<= 500` lines,
`publish = false` on all workspace crates, no hardcoded paths or secrets) also
remain mandatory across every module.

## Architecture & Module Rules

- **Ironic NestJS-style modules (`crates/server`)**: Organize each domain under `src/modules/<domain>/{mod.rs,controller/,services/,repositories/,dto/,entities/}`. Controllers stay thin; business logic lives in services; database queries live in repositories.
- **One-way dependencies**: `protocol` has zero workspace dependencies. `server`, `boxer`, `installer`, and `client` depend on `protocol`, never circularly on each other.
- **No third-party copying**: Never copy code, assets, or copy from studied repositories; reimplement clean strategies in idiomatic Rust.
- **Rust toolchain & security**: CI targets stable Rust (`1.100.0+` in `ubuntu-24.04`) with Cargo `min-publish-age` dependency cooldowns and OWASP Top 10:2025 / OWASP ASI 2026 guardrails (Argon2id + JWT, SHA-256 hashed opaque tokens, fail-closed sandbox isolation, `DropGuard` resource cleanup).
- **Strict English**: All tracked files, identifiers, comments, documentation, and commit messages are written in English.

## Branches

- `main` is the protected release branch; merges trigger tagging and release artifacts.
- `dev` is the protected integration branch where completed feature PRs land incrementally before consolidated releases to `main`.
- Work items live on short-lived branches (`feat/<scope>`, `fix/<scope>`, `docs/<scope>`, `ci/<scope>`) and reach `dev` (or `main`) exclusively through pull requests reviewed manually by `@leojara95`.

## Commits

Format: `type(scope): description`, lowercase, in English.

- Header has 3 to 7 words, counting `type(scope):`.
- Maximum 100 characters, no trailing period.
- Blank line after the header.
- At least 3 bullets formatted `- **Label**: body`, each body with 3 to 7 words.
- Allowed scopes: `client`, `server`, `boxer`, `installer`, `protocol`, `tools`, `docs`, `ci`, `workflow`, `hooks`, `release`, `license`, `deps`.
- Never include `Co-authored-by`.

## Releases

Versions derive from conventional commits since the last `v*` tag:

- `fix`, `perf`, and `revert` bump patch.
- `feat` bumps minor.
- `upgrade` or a breaking change bumps major.
- `docs`, `ci`, `chore`, `test`, `style`, and `refactor` do not trigger a release.

Before opening or updating a functional PR toward `main`, run `pnpm release:prepare` to update `package.json` and `CHANGELOG.md`, then verify with `pnpm release:check`.
