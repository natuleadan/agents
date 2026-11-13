#!/usr/bin/env node
// Generates ThirdPartyNotices.txt from direct Cargo and pnpm workspace dependencies.
// Usage: node tools/third-party-notices.mjs [--check]

import { execSync } from "node:child_process"
import { existsSync, readdirSync, readFileSync, statSync, writeFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..")
const OUTPUT = join(ROOT, "ThirdPartyNotices.txt")
const LICENSE_FILE_NAMES = [
  "LICENSE-APACHE",
  "LICENSE-MIT",
  "LICENSE",
  "LICENSE.md",
  "LICENSE.txt",
  "license",
  "license.md",
  "COPYING",
  "COPYING.md",
]

function repositoryUrl(raw) {
  if (!raw) return "unknown"
  return raw
    .replace(/^git\+/, "")
    .replace(/^git:\/\//, "https://")
    .replace(/\.git$/, "")
}

function readLicenseInDir(dir) {
  if (!dir || !existsSync(dir)) return null
  const found = []
  for (const name of LICENSE_FILE_NAMES) {
    const full = join(dir, name)
    if (existsSync(full) && statSync(full).isFile()) {
      found.push(readFileSync(full, "utf8").replace(/\r\n/g, "\n").trim())
    }
  }
  if (found.length > 0) return found[0]
  const entries = readdirSync(dir)
  for (const entry of entries) {
    const full = join(dir, entry)
    if (/^(license|copying)/i.test(entry) && statSync(full).isFile()) {
      return readFileSync(full, "utf8").replace(/\r\n/g, "\n").trim()
    }
  }
  return null
}

function repositoryBase(url) {
  return url.split("/").slice(0, 5).join("/")
}

function fallbackLicenseFor(rows, row) {
  const sameRepo = rows.find(
    (other) =>
      other.text &&
      other.license === row.license &&
      other.repository !== "unknown" &&
      repositoryBase(other.repository) === repositoryBase(row.repository),
  )
  if (sameRepo?.text) return sameRepo.text
  const sameLicense = rows.find(
    (other) => other.text && other.license === row.license,
  )
  if (sameLicense?.text) return sameLicense.text
  return `${row.license} — see ${row.repository}`
}

function collectRustDirectDeps() {
  const raw = execSync("cargo metadata --format-version=1", {
    cwd: ROOT,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  })
  const meta = JSON.parse(raw)
  const workspaceSet = new Set(meta.workspace_members ?? [])
  const packagesById = new Map(meta.packages.map((p) => [p.id, p]))
  const directByName = new Map()

  for (const wsId of workspaceSet) {
    const wsPkg = packagesById.get(wsId)
    if (!wsPkg) continue
    const wanted = new Set(
      (wsPkg.dependencies ?? [])
        .filter((d) => !d.path)
        .map((d) => d.name),
    )
    const node = meta.resolve?.nodes?.find((n) => n.id === wsId)
    for (const dep of node?.deps ?? []) {
      const target = packagesById.get(dep.pkg)
      if (!target || target.source === null || !wanted.has(target.name)) {
        continue
      }
      const key = `${target.name}@${target.version}`
      if (!directByName.has(key)) {
        const dir = dirname(target.manifest_path)
        directByName.set(key, {
          name: `${target.name} (cargo)`,
          version: target.version,
          license: target.license ?? "Unknown",
          repository: repositoryUrl(target.repository || target.homepage),
          text: readLicenseInDir(dir),
        })
      }
    }
  }
  return [...directByName.values()]
}

function collectNodeDirectDeps() {
  const manifestPath = join(ROOT, "package.json")
  if (!existsSync(manifestPath)) return []
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"))
  const deps = Object.keys({
    ...(manifest.dependencies ?? {}),
    ...(manifest.devDependencies ?? {}),
  }).sort()

  return deps.map((name) => {
    const dir = join(ROOT, "node_modules", name)
    if (!existsSync(dir)) {
      throw new Error(`missing node dependency ${name}; run pnpm install`)
    }
    const pkg = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"))
    const rawRepo =
      typeof pkg.repository === "string" ? pkg.repository : pkg.repository?.url
    return {
      name: `${name} (npm)`,
      version: pkg.version ?? "",
      license: pkg.license ?? "Unknown",
      repository: repositoryUrl(rawRepo),
      text: readLicenseInDir(dir),
    }
  })
}

const rows = [...collectRustDirectDeps(), ...collectNodeDirectDeps()].sort((a, b) =>
  a.name.localeCompare(b.name),
)

for (const row of rows) {
  if (!row.text) row.text = fallbackLicenseFor(rows, row)
}

const groups = new Map()
for (const row of rows) {
  if (!groups.has(row.text)) groups.set(row.text, [])
  groups.get(row.text).push(row.name)
}

const widths = {
  name: Math.max(7, ...rows.map((r) => r.name.length)),
  version: Math.max(7, ...rows.map((r) => r.version.length)),
  license: Math.max(7, ...rows.map((r) => r.license.length)),
}
const pad = (value, width) => value.padEnd(width, " ")

const table = [
  `${pad("Package", widths.name)}  ${pad("Version", widths.version)}  ${pad("License", widths.license)}  Repository`,
  `${"-".repeat(widths.name)}  ${"-".repeat(widths.version)}  ${"-".repeat(widths.license)}  ${"-".repeat(40)}`,
  ...rows.map(
    (r) =>
      `${pad(r.name, widths.name)}  ${pad(r.version, widths.version)}  ${pad(r.license, widths.license)}  ${r.repository}`,
  ),
].join("\n")

const licenses = [...groups.entries()]
  .map(([text, packageNames]) => {
    const title = text.split("\n")[0].trim()
    return [
      "==================================================",
      `  ${title}`,
      `  Applies to: ${packageNames.join(", ")}`,
      "==================================================",
      "",
      text,
    ].join("\n")
  })
  .join("\n\n")

const content = [
  "Third Party Notices",
  "",
  "Direct Rust and Node dependencies of the agents workspace.",
  "Generated by tools/third-party-notices.mjs; do not edit by hand.",
  "",
  table,
  "",
  "License texts",
  "",
  licenses,
  "",
].join("\n")

if (process.argv.includes("--check")) {
  const current = existsSync(OUTPUT) ? readFileSync(OUTPUT, "utf8") : ""
  if (current !== content) {
    console.error("FAIL: ThirdPartyNotices.txt out of date. Run `pnpm notices`.")
    process.exit(1)
  }
  console.log(`PASS: ThirdPartyNotices.txt up to date (${rows.length} packages).`)
} else {
  writeFileSync(OUTPUT, content)
  console.log(`Wrote ${rows.length} packages to ThirdPartyNotices.txt`)
}
