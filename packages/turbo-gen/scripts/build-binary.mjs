#!/usr/bin/env node

/**
 * Compiles the bundled dist/cli.js into a standalone binary using Node.js
 * Single Executable Applications (SEA).
 *
 * Prerequisites:
 *   - Node.js >= 20.0.0
 *   - `pnpm build` must have been run first to produce dist/cli.js
 *
 * The output binary is placed at dist/turbo-gen (or dist/turbo-gen.exe on
 * Windows).  It embeds the Node.js runtime + the fully-bundled CLI so that
 * there are zero CJS/ESM concerns at runtime — it's a single native
 * executable.
 *
 * Usage:
 *   node scripts/build-binary.mjs
 *   # => dist/turbo-gen  (or dist/turbo-gen.exe)
 */

import { execFileSync } from "node:child_process";
import { copyFileSync, chmodSync, existsSync } from "node:fs";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const pkgRoot = join(__dirname, "..");
const distDir = join(pkgRoot, "dist");

const isWindows = process.platform === "win32";
const isMac = process.platform === "darwin";
const binaryName = isWindows ? "turbo-gen.exe" : "turbo-gen";
const binaryPath = join(distDir, binaryName);
const seaConfig = join(pkgRoot, "sea-config.json");
const blobPath = join(distDir, "sea-prep.blob");

// ---------------------------------------------------------------------------
// 1. Verify that the bundle exists
// ---------------------------------------------------------------------------
if (!existsSync(join(distDir, "cli.js"))) {
  console.error("dist/cli.js not found — run `pnpm build` first.");
  process.exit(1);
}

// ---------------------------------------------------------------------------
// 2. Generate the SEA blob from the bundled JS
// ---------------------------------------------------------------------------
console.log("Generating SEA preparation blob…");
execFileSync(process.execPath, ["--experimental-sea-config", seaConfig], {
  cwd: pkgRoot,
  stdio: "inherit"
});

// ---------------------------------------------------------------------------
// 3. Copy the current Node.js binary — this becomes our binary
// ---------------------------------------------------------------------------
console.log("Copying Node.js binary…");
copyFileSync(process.execPath, binaryPath);

// Make it writable so we can inject the blob
if (!isWindows) {
  chmodSync(binaryPath, 0o755);
}

// ---------------------------------------------------------------------------
// 4. Remove the existing code signature (macOS only)
// ---------------------------------------------------------------------------
if (isMac) {
  console.log("Removing existing code signature (macOS)…");
  try {
    execFileSync("codesign", ["--remove-signature", binaryPath], {
      stdio: "inherit"
    });
  } catch {
    // codesign may not be available in CI linux containers — that's fine
  }
}

// ---------------------------------------------------------------------------
// 5. Inject the blob into the binary using postject
// ---------------------------------------------------------------------------
console.log("Injecting SEA blob into binary…");
const postjectArgs = [
  binaryPath,
  "NODE_SEA_BLOB",
  blobPath,
  "--sentinel-fuse",
  "NODE_SEA_FUSE_fce680ab2cc467b6e072b8b5df1996b2"
];

if (isMac) {
  postjectArgs.push("--macho-segment-name", "NODE_SEA");
}

// npx postject ships with Node.js >= 20; use it via the bundled copy
execFileSync("npx", ["postject", ...postjectArgs], {
  cwd: pkgRoot,
  stdio: "inherit",
  shell: isWindows
});

// ---------------------------------------------------------------------------
// 6. Re-sign the binary (macOS only)
// ---------------------------------------------------------------------------
if (isMac) {
  console.log("Re-signing binary (macOS)…");
  try {
    execFileSync("codesign", ["--sign", "-", binaryPath], {
      stdio: "inherit"
    });
  } catch {
    // optional — unsigned binaries still work
  }
}

console.log(`\nBinary compiled successfully: ${binaryPath}`);
console.log(
  `Size: ${(
    (await import("node:fs")).statSync(binaryPath).size /
    1024 /
    1024
  ).toFixed(1)} MB`
);
