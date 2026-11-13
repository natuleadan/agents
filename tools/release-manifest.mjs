#!/usr/bin/env node
// Ed25519 release manifest & OTA verification helper matching NSH updater format.
// Formats `<name>.pub` as raw 32-byte base64 (compatible with Rust `ed25519_dalek`) and `<name>.secret` as PKCS8 base64.
import { createHash, createPrivateKey, createPublicKey, generateKeyPairSync, sign, verify } from "node:crypto";
import { chmodSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const SPKI_ED25519_PREFIX = Buffer.from("302a300506032b6570032100", "hex");

export function keygen(prefixPath) {
  const { publicKey, privateKey } = generateKeyPairSync("ed25519");
  const spki = publicKey.export({ format: "der", type: "spki" });
  const rawPub = spki.subarray(spki.length - 32);
  const pkcs8 = privateKey.export({ format: "der", type: "pkcs8" });

  const pubPath = `${prefixPath}.pub`;
  const secPath = `${prefixPath}.secret`;
  mkdirSync(resolve(prefixPath, ".."), { recursive: true });
  writeFileSync(pubPath, `${rawPub.toString("base64")}\n`);
  writeFileSync(secPath, `${pkcs8.toString("base64")}\n`);
  chmodSync(secPath, 0o600);
  return { pubPath, secPath, pubB64: rawPub.toString("base64") };
}

function loadPrivateKey(secretB64) {
  const der = Buffer.from(secretB64.trim(), "base64");
  return createPrivateKey({ key: der, format: "der", type: "pkcs8" });
}

function loadPublicKey(pubB64) {
  const raw = Buffer.from(pubB64.trim(), "base64");
  const spki = raw.length === 32 ? Buffer.concat([SPKI_ED25519_PREFIX, raw]) : raw;
  return createPublicKey({ key: spki, format: "der", type: "spki" });
}

export function inferPlatformKey(filename) {
  const lower = filename.toLowerCase();
  if (lower.endsWith(".nshpkg")) return "__ota__";
  if (lower.includes("web-wasm")) return "web-wasm";
  if (lower.includes("android") || lower.endsWith(".apk") || lower.endsWith(".aab")) return "android-apk";
  if (lower.includes("ios") || lower.endsWith(".ipa")) return "ios-ipa";
  if (lower.includes("windows") || lower.endsWith(".exe") || lower.endsWith(".msi")) return "windows-x86_64";
  if ((lower.includes("macos") || lower.includes("darwin") || lower.endsWith(".dmg")) && lower.includes("x86_64")) {
    return "macos-x86_64";
  }
  if (lower.includes("macos") || lower.includes("darwin") || lower.endsWith(".dmg")) return "macos-aarch64";
  if (lower.includes("aarch64") || lower.includes("arm64")) return "linux-aarch64";
  if (lower.includes("linux") || lower.endsWith(".tar.gz")) return "linux-x86_64";
  return null;
}

export function sha256File(filePath) {
  const bytes = readFileSync(filePath);
  return {
    sha256: createHash("sha256").update(bytes).digest("hex"),
    size: statSync(filePath).size,
  };
}

export function createSignedManifest(opts) {
  const outDir = resolve(opts.outDir);
  mkdirSync(outDir, { recursive: true });
  const baseUrl = (opts.baseUrl || "").replace(/\/+$/, "");
  const platforms = {};
  const checksumLines = [];
  let pkgEntry = null;

  for (const rawFile of opts.files) {
    const file = resolve(rawFile);
    const name = basename(file);
    const { sha256, size } = sha256File(file);
    checksumLines.push(`${sha256}  ${name}`);

    const url = baseUrl ? `${baseUrl}/${name}` : name;
    const key = inferPlatformKey(name);
    if (key === "__ota__") {
      pkgEntry = {
        version: opts.version,
        build: Number(opts.build || 1),
        url,
        sha256,
        size,
        minRuntime: opts.minRuntime || "0.1.0",
      };
    } else if (key) {
      platforms[key] = { url, sha256, size };
    }
  }

  const manifest = {
    id: opts.appId,
    version: opts.version,
    notes: opts.notes || "",
    prerelease: Boolean(opts.prerelease),
    platforms,
    ...(pkgEntry ? { package: pkgEntry } : {}),
  };

  const manifestBytes = Buffer.from(`${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  const manifestPath = join(outDir, "latest.json");
  const sigPath = join(outDir, "latest.json.sig");
  const sumsPath = join(outDir, "SHA256SUMS");

  writeFileSync(manifestPath, manifestBytes);
  writeFileSync(sumsPath, `${checksumLines.sort().join("\n")}\n`);

  const priv = loadPrivateKey(opts.secretB64);
  const sigB64 = sign(null, manifestBytes, priv).toString("base64");
  writeFileSync(sigPath, `${sigB64}\n`);

  return { manifest, manifestPath, sigPath, sumsPath, sigB64 };
}

export function verifySignedManifest({ manifestPath, sigPath, pubB64, assetsDir }) {
  const manifestBytes = readFileSync(manifestPath);
  const sigRaw = Buffer.from(readFileSync(sigPath, "utf8").trim(), "base64");
  const pubKey = loadPublicKey(pubB64);
  if (!verify(null, manifestBytes, pubKey, sigRaw)) {
    throw new Error("Ed25519 signature verification failed on latest.json");
  }
  const manifest = JSON.parse(manifestBytes.toString("utf8"));
  if (assetsDir) {
    const entries = [...Object.values(manifest.platforms || {})];
    if (manifest.package) entries.push(manifest.package);
    for (const asset of entries) {
      const localName = basename(asset.url);
      const localPath = join(assetsDir, localName);
      const { sha256, size } = sha256File(localPath);
      if (sha256 !== asset.sha256) {
        throw new Error(`SHA-256 mismatch for ${localName}: expected ${asset.sha256}, got ${sha256}`);
      }
      if (size !== asset.size) {
        throw new Error(`Size mismatch for ${localName}: expected ${asset.size}, got ${size}`);
      }
    }
  }
  return manifest;
}

function parseCliArgs(argv) {
  const [cmd, ...rest] = argv;
  const flags = { _: [] };
  for (let i = 0; i < rest.length; i += 1) {
    const a = rest[i];
    if (a === "--prerelease") {
      flags.prerelease = true;
    } else if (a.startsWith("--") && i + 1 < rest.length) {
      flags[a.slice(2)] = rest[i + 1];
      i += 1;
    } else {
      flags._.push(a);
    }
  }
  return { cmd, flags };
}

const isEntrypoint = process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href;
if (isEntrypoint) {
  const { cmd, flags } = parseCliArgs(process.argv.slice(2));
  if (cmd === "keygen" && flags._[0]) {
    const res = keygen(flags._[0]);
    console.log(`Generated ${res.pubPath} and ${res.secPath}`);
  } else if (cmd === "sign-manifest") {
    const secretB64 = readFileSync(flags.key, "utf8");
    const res = createSignedManifest({
      appId: flags.id || "dev.natuleadan.agents",
      version: flags.version || "0.1.0",
      build: Number(flags.build || 1),
      minRuntime: flags["min-runtime"] || "0.1.0",
      notes: flags.notes || "",
      prerelease: Boolean(flags.prerelease),
      baseUrl: flags["base-url"] || "",
      secretB64,
      outDir: flags.out || "dist",
      files: flags._,
    });
    console.log(`Wrote signed ${res.manifestPath} and ${res.sigPath}`);
  } else if (cmd === "verify-manifest") {
    const pubB64 = readFileSync(flags.pub, "utf8");
    const m = verifySignedManifest({
      manifestPath: flags.manifest,
      sigPath: flags.sig,
      pubB64,
      assetsDir: flags["assets-dir"],
    });
    console.log(`PASS: verified manifest ${m.id} v${m.version} (${Object.keys(m.platforms || {}).length} platforms)`);
  } else {
    console.error("Usage: release-manifest.mjs <keygen|sign-manifest|verify-manifest> ...");
    process.exit(1);
  }
}
