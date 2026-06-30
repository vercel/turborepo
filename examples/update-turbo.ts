import { execSync } from "child_process";
import { readdirSync, existsSync, readFileSync, rmSync } from "fs";
import * as path from "path";
import { getPackageManagerInfo, getPackageManagerInstallCommand } from "./package-manager";

/** Script to refresh lockfiles after updating the "turbo" package across all examples */

const examplesDir = path.dirname(new URL(import.meta.url).pathname);
const commandEnv = {
  ...process.env,
  CI: "true",
  COREPACK_ENABLE_STRICT: "0",
  COREPACK_ENABLE_DOWNLOAD_PROMPT: "0"
};

function runCommand(command: string, cwd: string): void {
  execSync(command, { stdio: "inherit", cwd, env: commandEnv, shell: "/bin/bash" });
}

/** Get all directories in the examples folder */
const exampleDirs = readdirSync(examplesDir).filter((dir) =>
  existsSync(path.join(examplesDir, dir, "package.json"))
);

exampleDirs.forEach((dir) => {
  const packageJsonPath = path.join(examplesDir, dir, "package.json");

  try {
    const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf-8"));
    const { name: packageManager, version } = getPackageManagerInfo(packageJson);

    if (!packageJson.devDependencies?.turbo) {
      console.log(`Skipping ${dir} (no turbo dependency)...`);
      return;
    }

    const installCmd = getPackageManagerInstallCommand(packageManager, version, {
      updateLockfile: true
    });
    if (!installCmd) {
      throw new Error(`Unknown package manager "${packageManager}" in ${dir}`);
    }

    const cwd = path.join(examplesDir, dir);
    const nodeModulesPath = path.join(cwd, "node_modules");
    if (existsSync(nodeModulesPath)) {
      rmSync(nodeModulesPath, { recursive: true, force: true });
    }

    console.log(`Running ${installCmd} in ${dir}...`);
    runCommand(installCmd, cwd);
  } catch (error) {
    throw new Error(`Failed to process ${packageJsonPath}: ${error}`);
  }
});
