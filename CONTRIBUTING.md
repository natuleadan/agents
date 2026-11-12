# Contributing to Natuleadan Agents

> Read these conventions before contributing. This repository maintains strict commit
> and CI validation standards identical to `natuleadan/skills`.

## Quick start

1. Clone `https://github.com/natuleadan/agents.git`.
2. Run `pnpm install` to install commitlint and Husky hooks.
3. Run `cargo check --workspace` to verify the Rust workspace.
4. Run `pnpm release:check` to confirm the version baseline is clean.

## Architecture

The project consists of an isolated Cargo workspace with modular crates:

| Crate | Purpose |
|---|---|
| `crates/protocol` | Common message schemas, task DTOs and event contracts |
| `crates/server` | Orchestrator server with Ironic API, PostgreSQL, Valkey, NATS and autocron |
| `crates/boxer` | Sandbox container runner and resource supervisor for distributed agents |
| `crates/installer` | VPS provisioning and environment self-configuration CLI |
| `crates/client` | Multi-device client interface |

## Branches

`main` is the only permanent branch. Create a short branch per work item, open
a pull request and delete the branch after merge.

## Commit conventions

Format: `type(scope): description`. The scope is mandatory, in English and lower
case.

Scopes: `client`, `server`, `boxer`, `installer`, `protocol`, `tools`, `docs`,
`ci`, `workflow`, `hooks`, `release`, `license`, `deps`. CI rejects any other scope.

| Type | Use |
|---|---|
| `feat` | new capability or feature |
| `fix` | bug fix |
| `upgrade` | breaking change |
| `docs` | documentation |
| `style` | formatting only |
| `refactor` | internal restructuring |
| `perf` | performance improvement |
| `test` | tests |
| `chore` | configuration and tooling |
| `ci` | CI or hooks |
| `revert` | revert a previous commit |

Title and body rules:

- Title has 3 to 7 words, counting `type(scope):`.
- Maximum 100 characters, no trailing period.
- Blank line after the title.
- At least 3 bullets shaped `- **Label**: body`.
- Each bullet body has 3 to 7 words.

Example:

> docs(docs): explain commit rules and architecture
>
> - **Commits**: follow strict conventional policy
> - **Architecture**: distributed agent harness crates
> - **Branches**: protect main through pull requests

Rules live in `commitlint.config.js` and run in the `commit-msg` hook and in the
Conventional Commits CI job.

## Releases

Versions come from conventional commits since the last tag:

- `fix` and `perf` bump patch.
- `feat` bumps minor.
- `upgrade` or a breaking change bumps major.
- `docs`, `ci`, `chore`, `test`, `style` and `refactor` do not release.

A functional PR includes its own version bump and changelog entry:

1. Commit the change with `feat`, `fix` or `perf`.
2. Run `pnpm release:prepare`.
3. Amend the commit with the generated files.
4. Run `pnpm release:check`.

The PR is the only approval. After merge, the tag job creates `v<version>` and
publishes binary release artifacts. Version `0.0.0` means unreleased and is
skipped.
