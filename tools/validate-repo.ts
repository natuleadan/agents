#!/usr/bin/env node
// Enforces repository structural, modularity, and hygiene invariants.
import { readdirSync, readFileSync, existsSync, statSync } from "node:fs";
import { dirname, join, relative, extname } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const MAX_SOURCE_LINES = 500;
const SOURCE_EXTENSIONS = new Set([".rs", ".ts", ".sh"]);
const IGNORE_DIRS = new Set(["target", "node_modules", ".git", ".local", "internal", "dist"]);

const REQUIRED_FILES = [
  "AGENTS.md",
  "CHANGELOG.md",
  "CONTRIBUTING.md",
  "LICENSE",
  "NOTICE",
  "README.md",
  "SECURITY.md",
  ".env.example",
  ".gitattributes",
  ".gitignore",
  ".github/CODEOWNERS",
  ".github/dependabot.yml",
  ".github/workflows/validate.yml",
  "commitlint.config.js",
  "package.json",
  "Cargo.toml",
];

const FORBIDDEN_PATTERNS: Array<{ name: string; regex: RegExp }> = [
  { name: "macOS user path", regex: /\/Users\/[A-Za-z0-9._-]+\// },
  { name: "Linux workspace user path", regex: /\/home\/antigravity\// },
  { name: "Co-authored-by trailer", regex: /Co-authored-by:/i },
  { name: "PEM private key header", regex: /-----BEGIN (?:OPENSSH |RSA |EC )?PRIVATE KEY-----/ },
  {
    name: "third-party inspiration or internal provider reference",
    regex: /\b(?:opencode|n8n|jcode|pi-mono|sdk-ops|herdr|codex-rs|claude-code|cubepath)\b/i,
  },
];

let failures = 0;

function fail(msg: string) {
  console.error(`FAIL: ${msg}`);
  failures += 1;
}

for (const rel of REQUIRED_FILES) {
  if (!existsSync(join(ROOT, rel))) {
    fail(`Missing required governance/repo file: ${rel}`);
  }
}

function walk(dir: string) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (IGNORE_DIRS.has(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      walk(full);
      continue;
    }
    if (!entry.isFile()) continue;

    const rel = relative(ROOT, full);
    const ext = extname(entry.name);

    if (rel.startsWith("crates/") && entry.name === "Cargo.toml") {
      const manifest = readFileSync(full, "utf8");
      if (!/^publish\s*=\s*false\b/m.test(manifest)) {
        fail(`${rel} must set publish = false to prevent accidental crates.io publishing`);
      }
    }

    if (SOURCE_EXTENSIONS.has(ext) && (rel.startsWith("crates/") || rel.startsWith("tools/") || rel.startsWith("deploy/"))) {
      const content = readFileSync(full, "utf8");
      const lines = content.split("\n").length;
      if (lines > MAX_SOURCE_LINES) {
        fail(`${rel} has ${lines} lines (max ${MAX_SOURCE_LINES} per modular monolith rule)`);
      }
    }

    if (
      rel !== "tools/validate-repo.ts" &&
      rel !== "pnpm-lock.yaml" &&
      rel !== "Cargo.lock" &&
      statSync(full).size < 1_000_000
    ) {
      const text = readFileSync(full, "utf8");
      for (const pattern of FORBIDDEN_PATTERNS) {
        if (pattern.regex.test(text)) {
          fail(`${rel} matches forbidden pattern (${pattern.name})`);
        }
      }
    }
  }
}

walk(ROOT);

if (failures > 0) {
  console.error(`FAIL: ${failures} validation error(s) found.`);
  process.exit(1);
}

console.log("PASS: repository governance, line limits, and hygiene checks succeeded.");
