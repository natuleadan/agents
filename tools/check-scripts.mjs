#!/usr/bin/env node
// Validates all shell provisioning scripts in deploy/scripts/.
// Usage: node tools/check-scripts.mjs

import { execFileSync, spawnSync } from "node:child_process"
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..")
const SCRIPTS_DIR = join(ROOT, "deploy/scripts")
const MAX_LINES = 500
const EXPECTED_SCRIPTS = [
  "00-preflight.sh",
  "10-hardening.sh",
  "20-deps-server.sh",
  "30-deps-boxer.sh",
  "40-systemd-units.sh",
  "90-doctor.sh",
]

if (!existsSync(SCRIPTS_DIR)) {
  console.error("FAIL: missing deploy/scripts directory")
  process.exit(1)
}

const files = readdirSync(SCRIPTS_DIR)
  .filter((name) => name.endsWith(".sh"))
  .sort()

for (const req of EXPECTED_SCRIPTS) {
  if (!files.includes(req)) {
    console.error(`FAIL: missing expected script deploy/scripts/${req}`)
    process.exit(1)
  }
}

const paths = files.map((name) => join(SCRIPTS_DIR, name))

for (const path of paths) {
  const rel = path.replace(`${ROOT}/`, "")
  const lines = readFileSync(path, "utf8").split("\n").length
  if (lines > MAX_LINES) {
    console.error(`FAIL: ${rel} has ${lines} lines (max ${MAX_LINES})`)
    process.exit(1)
  }
  const mode = statSync(path).mode & 0o111
  if (mode === 0) {
    console.error(`FAIL: ${rel} must be executable (+x)`)
    process.exit(1)
  }
  execFileSync("bash", ["-n", path], { stdio: "inherit" })
}

const sc = spawnSync("shellcheck", ["-S", "warning", ...paths], { stdio: "inherit" })
if (sc.error) {
  if (sc.error.code === "ENOENT") {
    console.warn("SKIP: shellcheck not found in PATH; skipping shellcheck lint on this host.")
  } else {
    console.error(`FAIL: unable to invoke shellcheck: ${sc.error.message}`)
    process.exit(1)
  }
} else if (sc.status !== 0) {
  process.exit(sc.status ?? 1)
}

const shfmtCheck = spawnSync("shfmt", ["-d", "-i", "2", "-ci", ...paths], { stdio: "inherit" })
if (!shfmtCheck.error && shfmtCheck.status !== 0) {
  console.error("FAIL: shfmt formatting check failed")
  process.exit(shfmtCheck.status ?? 1)
}

console.log(
  `PASS: ${files.length} deployment script(s) validated (syntax, permissions, line limit, shellcheck${shfmtCheck.error ? "" : ", shfmt"}).`,
)
