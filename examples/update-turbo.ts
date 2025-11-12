import { execSync } from "child_process";
import { readdirSync, existsSync, readFileSync } from "fs";
import * as path from "path";

/** Script to update the "turbo" package across all example directories */

const examplesDir = path.resolve(__dirname);

/** Get all directories in the examples folder */
const exampleDirs = readdirSync(examplesDir).filter((dir) =>
  existsSync(path.join(examplesDir, dir, "package.json"))
);

exampleDirs.forEach((dir) => {
  const packageJsonPath = path.join(examplesDir, dir, "package.json");

  try {
    const packageJson = JSON.parse(readFileSync(packageJsonPath, "utf-8"));

    // Check the packageManager field and run the correct update command
    const packageManager: string = packageJson.packageManager;
    if (!packageManager) {
      throw new Error(`Missing packageManager field in ${packageJsonPath}`);
    }

    let updateCmd: string;

    if (packageManager.startsWith("pnpm")) {
      updateCmd = "pnpm update turbo";
    } else if (packageManager.startsWith("yarn")) {
      // Extract version from packageManager field (e.g., "yarn@1.22.19" -> "1")
      const yarnVersion = packageManager.split("@")[1]?.split(".")[0];
      if (yarnVersion && parseInt(yarnVersion, 10) >= 2) {
        // Yarn Berry (2.x+) uses "up" command
        updateCmd = "yarn up turbo";
      } else {
        // Yarn Classic (1.x) uses "upgrade" command
        updateCmd = "yarn upgrade turbo";
      }
    } else if (packageManager.startsWith("npm")) {
      updateCmd = "npm update turbo";
    } else {
      throw new Error(`Unknown package manager "${packageManager}" in ${dir}`);
    }

    console.log(`Running ${updateCmd} in ${dir}...`);
    execSync(updateCmd, {
      stdio: "inherit",
      cwd: path.join(examplesDir, dir),
    });
  } catch (error) {
    throw new Error(`Failed to process ${packageJsonPath}: ${error}`);
  }
});
