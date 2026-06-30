import { execSync } from "child_process";
import { readdirSync, existsSync, readFileSync } from "fs";
import * as path from "path";
import { getPackageManagerInfo, getPackageManagerInstallCommand } from "./package-manager";

/** Note: this script intentionally doesn't run during regular `pnpm install` from the project root because it's not something we expect to need to do all the time and integrating it into the project install flow is excessive */

const examplesDir = path.dirname(new URL(import.meta.url).pathname);
const corepackEnv = {
  ...process.env,
  CI: "true",
  COREPACK_ENABLE_STRICT: "0",
  COREPACK_ENABLE_DOWNLOAD_PROMPT: "0"
};

function runCommand(command: string, cwd: string): void {
  execSync(command, { stdio: "inherit", cwd, env: corepackEnv, shell: "/bin/bash" });
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

    const installCmd = getPackageManagerInstallCommand(packageManager, version);
    if (!installCmd) {
      throw new Error(`Unknown package manager "${packageManager}" in ${dir}`);
    }

    const cwd = path.join(examplesDir, dir);
    console.log(`Running ${installCmd} in ${dir}...`);
    runCommand(installCmd, cwd);
  } catch (error) {
    throw new Error(`Failed to process ${packageJsonPath}: ${error}`);
  }
});
