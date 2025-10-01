import { execSync } from "child_process";
import { readdirSync, existsSync, readFileSync } from "fs";
import * as path from "path";

/** Note: this script intentionally doesn't run during regular `pnpm install` from the project root because it's not something we expect to need to do all the time and integrating it into the project install flow is excessive */

const examplesDir = path.resolve(__dirname);

/** Get all directories in the examples folder */
const exampleDirs = readdirSync(examplesDir).filter((dir) =>
  existsSync(path.join(examplesDir, dir, "package.json"))
);

exampleDirs.forEach((dir) => {
  const packageJsonPath = path.join(examplesDir, dir, "package.json");

  try {
    const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf-8"));

    // Check the packageManager field and run the correct install command
    const packageManager: string = packageJson.packageManager;
    if (!packageManager) {
      throw new Error(`Missing packageManager field in ${packageJsonPath}`);
    }

    let installCmd: string;

    if (packageManager.startsWith("pnpm")) {
      installCmd = "pnpm install";
    } else if (packageManager.startsWith("yarn")) {
      installCmd = "yarn install";
    } else if (packageManager.startsWith("npm")) {
      installCmd = "npm install";
    } else {
      throw new Error(`Unknown package manager "${packageManager}" in ${dir}`);
    }

    console.log(`Running ${installCmd} in ${dir}...`);
    execSync(installCmd, {
      stdio: "inherit",
      cwd: path.join(examplesDir, dir),
    });
  } catch (error) {
    throw new Error(`Failed to process ${packageJsonPath}: ${error}`);
  }
});
