import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import { createSignedManifest, keygen, verifySignedManifest } from "./release-manifest.mjs";

test("release-manifest signs, verifies, and detects tampered manifests or artifacts", () => {
  const dir = mkdtempSync(join(tmpdir(), "agents-manifest-test-"));
  try {
    const { pubPath, secPath } = keygen(join(dir, "keys/update"));
    mkdirSync(join(dir, "dist"), { recursive: true });
    const linuxTar = join(dir, "dist/agents-client-linux-x86_64.tar.gz");
    const macDmg = join(dir, "dist/agents-client-macos-aarch64.dmg");
    const otaPkg = join(dir, "dist/dev.natuleadan.agents-2.nshpkg");
    writeFileSync(linuxTar, "linux-binary-contents");
    writeFileSync(macDmg, "macos-dmg-contents");
    writeFileSync(otaPkg, "signed-ota-bundle");

    const secretB64 = readFileSync(secPath, "utf8");
    const pubB64 = readFileSync(pubPath, "utf8");
    const res = createSignedManifest({
      appId: "dev.natuleadan.agents",
      version: "0.2.0",
      build: 2,
      minRuntime: "0.1.0",
      notes: "Automated release build",
      prerelease: false,
      baseUrl: "https://github.com/natuleadan/agents/releases/download/v0.2.0",
      secretB64,
      outDir: join(dir, "dist"),
      files: [linuxTar, macDmg, otaPkg],
    });

    const verified = verifySignedManifest({
      manifestPath: res.manifestPath,
      sigPath: res.sigPath,
      pubB64,
      assetsDir: join(dir, "dist"),
    });
    assert.equal(verified.id, "dev.natuleadan.agents");
    assert.equal(verified.version, "0.2.0");
    assert.equal(verified.package?.build, 2);
    assert.ok(verified.platforms["linux-x86_64"]);
    assert.ok(verified.platforms["macos-aarch64"]);

    // Tamper manifest and assert signature rejection
    const orig = readFileSync(res.manifestPath, "utf8");
    writeFileSync(res.manifestPath, orig.replace("0.2.0", "0.9.9"));
    assert.throws(
      () =>
        verifySignedManifest({
          manifestPath: res.manifestPath,
          sigPath: res.sigPath,
          pubB64,
        }),
      /Ed25519 signature verification failed/,
    );
    writeFileSync(res.manifestPath, orig);

    // Tamper artifact and assert SHA-256 rejection
    writeFileSync(linuxTar, "corrupted");
    assert.throws(
      () =>
        verifySignedManifest({
          manifestPath: res.manifestPath,
          sigPath: res.sigPath,
          pubB64,
          assetsDir: join(dir, "dist"),
        }),
      /SHA-256 mismatch/,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
