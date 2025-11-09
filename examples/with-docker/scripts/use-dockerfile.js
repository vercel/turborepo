#!/usr/bin/env node

const fs = require("fs");
const path = require("path");
const readline = require("readline");
const { execSync } = require("child_process");

const [, , tool] = process.argv;

if (!tool || !["npm", "pnpm", "yarn"].includes(tool)) {
  console.error("Usage: node use-dockerfile.js <npm|pnpm|yarn>");
  process.exit(1);
}

const targets = [
  {
    app: "web",
    target: path.resolve(__dirname, "../apps/web/Dockerfile"),
    source: path.resolve(
      __dirname,
      `../apps/web/dockerfile-examples/${tool}/Dockerfile`
    ),
    archiveDir: path.resolve(__dirname, "../apps/web/archived-dockerfile"),
  },
  {
    app: "api",
    target: path.resolve(__dirname, "../apps/api/Dockerfile"),
    source: path.resolve(
      __dirname,
      `../apps/api/dockerfile-examples/${tool}/Dockerfile`
    ),
    archiveDir: path.resolve(__dirname, "../apps/api/archived-dockerfile"),
  },
];

// Hardcoded versions for package managers
const packageManagerVersions = {
  pnpm: "pnpm@10.10.0",
  npm: "npm@10.5.0",
  yarn: "yarn@3.7.0",
};

function getPackageManagerVersion(tool) {
  return packageManagerVersions[tool] || `${tool}@latest`;
}

function getTimestamp() {
  const now = new Date();
  return now.toISOString().replace(/:/g, "-").replace(/\..+/, ""); // Remove milliseconds
}

const toolVersion = getPackageManagerVersion(tool);

console.warn(`
⚠️  WARNING: This script will:
- ❌ DELETE 🗑️ your existing Dockerfiles in:
  - examples/with-docker/apps/api/Dockerfile
  - examples/with-docker/apps/web/Dockerfile
- 📦 ARCHIVE them to:
  - examples/with-docker/apps/api/archived-dockerfile/
  - examples/with-docker/apps/web/archived-dockerfile/
- REPLACE them with the  ✨  ${tool} ✨ Dockerfiles
- UPDATE the "packageManager" field in examples/with-docker/package.json to "${toolVersion}"
- CLEAN existing node_modules and lock files
- ENABLE Corepack if needed (may require sudo password)

Are you sure you want to continue? (y/n)
`);

const rl = readline.createInterface({
  input: process.stdin,
  output: process.stdout,
});

rl.question("> ", (answer) => {
  if (answer.toLowerCase() !== "y") {
    console.log("Aborting.");
    rl.close();
    process.exit(0);
  }

  // Create archive directories and archive/delete Dockerfiles
  targets.forEach(({ target, source, app, archiveDir }) => {
    if (fs.existsSync(target)) {
      // Create archive directory if it doesn't exist
      if (!fs.existsSync(archiveDir)) {
        fs.mkdirSync(archiveDir, { recursive: true });
        console.log(`Created archive directory: ${archiveDir}`);
      }

      // Archive the existing Dockerfile with timestamp
      const timestamp = getTimestamp();
      const archivePath = path.join(archiveDir, `Dockerfile-${timestamp}`);
      fs.copyFileSync(target, archivePath);
      console.log(`Archived ${target} to ${archivePath}`);

      // Delete the original file
      fs.unlinkSync(target);
      console.log(`Deleted ${target}`);
    }
    fs.copyFileSync(source, target);
    console.log(`Copied ${tool} Dockerfile for ${app}`);
  });

  // Update the packageManager field
  const packageJsonPath = path.resolve(__dirname, "../package.json");
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));

  packageJson.packageManager = toolVersion;
  fs.writeFileSync(packageJsonPath, JSON.stringify(packageJson, null, 2));
  console.log(`Updated packageManager in package.json to "${toolVersion}"`);

  // Clean up existing node_modules and lock files
  try {
    console.log("\n🧹 Cleaning up existing node_modules and lock files...");
    const projectRoot = path.resolve(__dirname, "..");

    // Remove node_modules
    if (fs.existsSync(path.join(projectRoot, "node_modules"))) {
      execSync(`rm -rf ${path.join(projectRoot, "node_modules")}`, {
        stdio: "inherit",
      });
    }

    // Remove lock files that don't match the current package manager
    if (
      tool !== "pnpm" &&
      fs.existsSync(path.join(projectRoot, "pnpm-lock.yaml"))
    ) {
      fs.unlinkSync(path.join(projectRoot, "pnpm-lock.yaml"));
    }
    if (
      tool !== "npm" &&
      fs.existsSync(path.join(projectRoot, "package-lock.json"))
    ) {
      fs.unlinkSync(path.join(projectRoot, "package-lock.json"));
    }
    if (tool !== "yarn") {
      if (fs.existsSync(path.join(projectRoot, "yarn.lock"))) {
        fs.unlinkSync(path.join(projectRoot, "yarn.lock"));
      }
      // Also remove .yarn directory if it exists
      if (fs.existsSync(path.join(projectRoot, ".yarn"))) {
        execSync(`rm -rf ${path.join(projectRoot, ".yarn")}`, {
          stdio: "inherit",
        });
      }
    } else {
      // For Yarn, create an empty yarn.lock file if it doesn't exist
      // This helps Yarn recognize the project directory
      if (!fs.existsSync(path.join(projectRoot, "yarn.lock"))) {
        fs.writeFileSync(path.join(projectRoot, "yarn.lock"), "");
        console.log(
          "Created empty yarn.lock file to help Yarn recognize the project"
        );
      }
    }

    console.log("✅ Cleanup completed!");
  } catch (err) {
    console.error(`⚠️ Error during cleanup: ${err.message}`);
  }

  // Run install command
  try {
    console.log(`\n📦 Installing dependencies using ${tool}...`);

    // Special handling for Yarn
    if (tool === "yarn") {
      try {
        console.log("Setting up Yarn environment...");

        // Make sure parent directories don't have conflicting yarn.lock files
        const homeDir = process.env.HOME || process.env.USERPROFILE;
        const possibleParentYarnLock = path.join(homeDir, "yarn.lock");
        let parentYarnLockBackedUp = false;

        if (fs.existsSync(possibleParentYarnLock)) {
          // Temporarily rename parent yarn.lock to avoid conflicts
          fs.renameSync(
            possibleParentYarnLock,
            `${possibleParentYarnLock}.bak`
          );
          parentYarnLockBackedUp = true;
          console.log("Temporarily backed up parent yarn.lock file");
        }

        // Enable Corepack
        console.log("Enabling Corepack for Yarn...");
        try {
          // Try with sudo first
          try {
            console.log(
              "Attempting to enable Corepack with sudo (you may be prompted for password)..."
            );
            execSync("sudo corepack enable", {
              stdio: "inherit",
            });
          } catch (sudoErr) {
            // If sudo fails, try without sudo
            console.log("Attempting to enable Corepack without sudo...");
            execSync("corepack enable", {
              stdio: "inherit",
            });
          }

          // Prepare Yarn with the specific version
          console.log("Preparing Yarn...");
          execSync("corepack prepare yarn@3.7.0 --activate", {
            cwd: path.resolve(__dirname, ".."),
            stdio: "inherit",
          });

          // Create .yarnrc.yml if it doesn't exist
          const yarnrcPath = path.join(
            path.resolve(__dirname, ".."),
            ".yarnrc.yml"
          );
          if (!fs.existsSync(yarnrcPath)) {
            fs.writeFileSync(yarnrcPath, "nodeLinker: node-modules\n");
            console.log(
              "Created .yarnrc.yml file with nodeLinker configuration"
            );
          }

          // Run yarn install with the --no-immutable flag to allow changes
          console.log("Running yarn install...");
          execSync("yarn install --no-immutable", {
            cwd: path.resolve(__dirname, ".."),
            stdio: "inherit",
            env: { ...process.env, YARN_ENABLE_IMMUTABLE_INSTALLS: "false" },
          });

          // Restore parent yarn.lock if we backed it up
          if (parentYarnLockBackedUp) {
            fs.renameSync(
              `${possibleParentYarnLock}.bak`,
              possibleParentYarnLock
            );
            console.log("Restored parent yarn.lock file");
          }

          console.log("✅ Yarn setup completed!");
        } catch (corepackErr) {
          console.error(`⚠️ Corepack setup failed: ${corepackErr.message}`);
          console.warn(
            "Continuing with alternative yarn installation method..."
          );

          // Try using yarn directly as a fallback
          try {
            execSync("yarn install --no-immutable", {
              cwd: path.resolve(__dirname, ".."),
              stdio: "inherit",
              env: { ...process.env, YARN_ENABLE_IMMUTABLE_INSTALLS: "false" },
            });
          } catch (directYarnErr) {
            throw new Error(
              `Failed to install with yarn directly: ${directYarnErr.message}`
            );
          }
        }
      } catch (yarnSetupErr) {
        throw new Error(`Yarn setup failed: ${yarnSetupErr.message}`);
      }
    } else {
      // For npm and pnpm, use the standard install command
      execSync(`${tool} install`, {
        cwd: path.resolve(__dirname, ".."),
        stdio: "inherit",
      });
    }

    console.log("✅ Dependencies installed!");
  } catch (err) {
    console.error(`❌ Failed to run ${tool} install. Please run it manually.`);
    console.error(`   Error details: ${err.message}`);

    if (tool === "yarn") {
      console.error(
        "   To fix this issue, please run these commands manually:"
      );
      console.error("   1. sudo corepack enable");
      console.error("   2. corepack prepare yarn@3.7.0 --activate");
      console.error("   3. cd to the project directory");
      console.error("   4. yarn install --no-immutable");
    }
  }

  console.log("✅ Done!");
  rl.close();
});
