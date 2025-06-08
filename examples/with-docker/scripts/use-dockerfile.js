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
  },
  {
    app: "api",
    target: path.resolve(__dirname, "../apps/api/Dockerfile"),
    source: path.resolve(
      __dirname,
      `../apps/api/dockerfile-examples/${tool}/Dockerfile`
    ),
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

const toolVersion = getPackageManagerVersion(tool);

console.warn(`
⚠️  WARNING: This script will:
- ❌ DELETE 🗑️ your existing Dockerfiles in:
  - examples/with-docker/apps/api/Dockerfile
  - examples/with-docker/apps/web/Dockerfile
- REPLACE them with the  ✨  ${tool} ✨ Dockerfiles
- UPDATE the "packageManager" field in examples/with-docker/package.json to "${toolVersion}"

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

  // Delete and copy Dockerfiles
  targets.forEach(({ target, source, app }) => {
    if (fs.existsSync(target)) {
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

  // Run install command
  try {
    console.log(`\n📦 Installing dependencies using ${tool}...`);
    execSync(`${tool} install`, {
      cwd: path.resolve(__dirname, ".."),
      stdio: "inherit",
    });
    console.log("✅ Dependencies installed!");
  } catch (err) {
    console.error(`❌ Failed to run ${tool} install. Please run it manually.`);
  }

  console.log("✅ Done!");
  rl.close();
});
