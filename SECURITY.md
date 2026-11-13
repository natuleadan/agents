# Security Policy

## Supported versions

The latest release on `main` receives security fixes. Older versions are not
supported.

## Reporting a vulnerability

Do not open a public issue. Use the private advisory form:

https://github.com/natuleadan/agents/security/advisories/new

Include the affected crate or component, a minimal reproduction and the impact.
If the report involves a dependency or sandbox boundary, include the crate name,
version and target environment.

## Response

We acknowledge the report, triage it and coordinate disclosure after a fix is
available.

## Scope

This repository ships the distributed agent harness workspace (`agents-protocol`,
`agents-server`, `agents-boxer`, `agents-installer`, `agents-client`) and
deployment hardening scripts. Never commit live tokens, keys or `.env` files.
