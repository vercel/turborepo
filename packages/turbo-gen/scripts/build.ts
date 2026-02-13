/**
 * Build script for @turbo/gen.
 *
 * 1. Compiles the CLI into a standalone Bun binary for each target platform
 * 2. Generates .d.ts for the types-only export (PlopTypes re-export)
 *
 * Template embedding is handled by the `build:embed` turbo task
 * which runs before this script via task dependencies.
 *
 * Run: bun run scripts/build.ts
 * Cross-compile: bun run scripts/build.ts --all-platforms
 */

import { $ } from "bun";
import path from "node:path";
import fs from "node:fs";

const ROOT = path.join(import.meta.dir, "..");
const DIST = path.join(ROOT, "dist");
const ENTRY = path.join(ROOT, "src", "cli.ts");

interface Platform {
  target: string;
  outfile: string;
}

const CURRENT_PLATFORM: Platform = (() => {
  const os = process.platform === "win32" ? "windows" : process.platform;
  const arch = process.arch === "x64" ? "x64" : "arm64";
  const ext = os === "windows" ? ".exe" : "";
  return {
    target: `bun-${os}-${arch}`,
    outfile: `turbo-gen${ext}`
  };
})();

const ALL_PLATFORMS: Array<Platform> = [
  { target: "bun-darwin-arm64", outfile: "turbo-gen-darwin-arm64" },
  { target: "bun-darwin-x64", outfile: "turbo-gen-darwin-x64" },
  { target: "bun-linux-x64", outfile: "turbo-gen-linux-x64" },
  { target: "bun-linux-arm64", outfile: "turbo-gen-linux-arm64" },
  { target: "bun-windows-x64", outfile: "turbo-gen-windows-x64.exe" }
];

async function compileBinary(platform: Platform) {
  const outPath = path.join(DIST, platform.outfile);
  console.log(`Compiling ${platform.target} → ${platform.outfile}`);
  await $`bun build ${ENTRY} --compile --target=${platform.target} --outfile=${outPath}`
    .cwd(ROOT)
    .quiet();
  const stat = fs.statSync(outPath);
  console.log(`  ${(stat.size / 1024 / 1024).toFixed(1)} MB`);
}

async function generateTypes() {
  console.log("Generating types via tsdown...");
  await $`pnpm tsdown`.cwd(ROOT);
}

async function main() {
  const allPlatforms = process.argv.includes("--all-platforms");

  fs.mkdirSync(DIST, { recursive: true });

  if (allPlatforms) {
    console.log(`\nCross-compiling for ${ALL_PLATFORMS.length} platforms...`);
    for (const platform of ALL_PLATFORMS) {
      await compileBinary(platform);
    }
  } else {
    console.log(`\nCompiling for current platform...`);
    await compileBinary(CURRENT_PLATFORM);
  }

  await generateTypes();

  console.log("\nBuild complete.");
}

main().catch((e) => {
  console.error("Build failed:", e);
  process.exit(1);
});
