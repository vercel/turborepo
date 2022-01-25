import fs from "fs-extra";
import path from "path";
import { Flags } from "../types";
import chalk from "chalk";

export default function createTurboConfig(files: string[], flags: Flags) {
  if (files.length === 1) {
    const dir = files[0];
    const root = path.resolve(process.cwd(), dir);
    console.log(`Migrating package.json "turbo" key to "turbo.json" file...`);
    const turboConfigPath = path.join(root, "turbo.json");

    const rootPackageJsonPath = path.join(root, "package.json");
    if (!fs.existsSync(rootPackageJsonPath)) {
      error(`No package.json found at ${root}. Is the path correct?`);
      process.exit(1);
    }
    const rootPackageJson = fs.readJsonSync(rootPackageJsonPath);

    if (fs.existsSync(turboConfigPath)) {
      skip("turbo.json already exists");
      return;
    }

    if (rootPackageJson.hasOwnProperty("turbo")) {
      const { turbo: turboConfig, ...remainingPkgJson } = rootPackageJson;
      if (flags.dry) {
        skip("Skipping writing turbo.json (dry run)");
        skip('Skipping deleting "turbo" key from package.json (dry run)');
      } else {
        console.log("Writing turbo.json");
        fs.writeJsonSync(turboConfigPath, turboConfig, { spaces: 2 });
        console.log('Removing "turbo" key from package.json');
        fs.writeJsonSync(rootPackageJsonPath, remainingPkgJson, { spaces: 2 });
      }
    } else {
      skip('"turbo" key does not exist in package.json');
    }
    ok("Finished");
  }
}

function skip(...args: any[]) {
  console.log(chalk.yellow.inverse(` SKIP `), ...args);
}
function error(...args: any[]) {
  console.log(chalk.red.inverse(` ERROR `), ...args);
}
function ok(...args: any[]) {
  console.log(chalk.green.inverse(` OK `), ...args);
}
