#!/usr/bin/env bun

import fs from "fs/promises";
import path from "path";
import { exec } from "child_process";
import { promisify } from "util";

const execAsync = promisify(exec);

async function syncTemplates() {
  const templateDir = path.join(process.cwd(), "template");
  const evalsDir = path.join(process.cwd(), "evals");

  // Read template files
  const templatePackageJson = await fs.readFile(
    path.join(templateDir, "package.json"),
    "utf-8"
  );

  const templateTurboJson = await fs.readFile(
    path.join(templateDir, "turbo.json"),
    "utf-8"
  );

  const templatePnpmWorkspace = await fs.readFile(
    path.join(templateDir, "pnpm-workspace.yaml"),
    "utf-8"
  );

  // Get all eval directories
  const entries = await fs.readdir(evalsDir, { withFileTypes: true });

  let updatedCount = 0;
  let skippedCount = 0;

  console.log("📋 Syncing template files to all evals...\n");

  for (const entry of entries) {
    if (entry.isDirectory() && /^\d+/.test(entry.name)) {
      const evalName = entry.name;
      const inputDir = path.join(evalsDir, evalName, "input");

      // Check if input directory exists
      const inputExists = await fs
        .stat(inputDir)
        .then((s) => s.isDirectory())
        .catch(() => false);

      if (!inputExists) {
        console.log(`⏭️  ${evalName}: Skipped (no input directory)`);
        skippedCount++;
        continue;
      }

      try {
        // Copy pnpm-workspace.yaml
        await fs.writeFile(
          path.join(inputDir, "pnpm-workspace.yaml"),
          templatePnpmWorkspace,
          "utf-8"
        );

        console.log(`✅ ${evalName}: Synced pnpm-workspace.yaml`);
        updatedCount++;
      } catch (error: any) {
        console.log(`⏭️  ${evalName}: Skipped (${error.message})`);
        skippedCount++;
      }
    }
  }

  console.log(`\n📊 Sync Summary:`);
  console.log(`   ${updatedCount} evals updated`);
  console.log(`   ${skippedCount} evals skipped`);

  // Check if evals/package.json needs updating
  console.log("\n🔍 Checking shared dependencies...");

  const evalsPackageJsonPath = path.join(evalsDir, "package.json");
  let needsInstall = false;

  try {
    const evalsPackageJson = await fs.readFile(evalsPackageJsonPath, "utf-8");

    if (evalsPackageJson !== templatePackageJson) {
      console.log("   📦 evals/package.json differs from template");
      needsInstall = true;
    } else {
      console.log("   ✓ evals/package.json matches template");
    }
  } catch (error) {
    console.log("   📦 evals/package.json not found");
    needsInstall = true;
  }

  if (needsInstall) {
    console.log("\n🧹 Cleaning up shared dependencies...");

    // Remove node_modules
    try {
      await fs.rm(path.join(evalsDir, "node_modules"), {
        recursive: true,
        force: true
      });
      console.log("   ✓ Removed evals/node_modules");
    } catch (error) {
      // Ignore if doesn't exist
    }

    // Copy template package.json to evals/
    await fs.writeFile(evalsPackageJsonPath, templatePackageJson, "utf-8");
    console.log("   ✓ Updated evals/package.json");

    // Run pnpm install
    console.log("\n📦 Installing shared dependencies...");
    console.log("   Running: pnpm install --prefer-offline");

    try {
      const { stdout, stderr } = await execAsync(
        `cd "${evalsDir}" && pnpm install --prefer-offline`,
        { maxBuffer: 10 * 1024 * 1024 }
      );

      if (stdout) console.log(stdout);
      if (stderr) console.error(stderr);

      console.log("   ✅ Dependencies installed successfully");
    } catch (error: any) {
      console.error("   ❌ Failed to install dependencies:");
      console.error(error.message);
      process.exit(1);
    }
  }

  console.log("\n🎉 Done! All templates synced and dependencies up to date.");
}

syncTemplates().catch((error) => {
  console.error("Error:", error);
  process.exit(1);
});
