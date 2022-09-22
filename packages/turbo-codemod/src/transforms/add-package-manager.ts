import { Flags } from "../types";
import path from "path";
import { getWorkspaceImplementation } from "../getWorkspaceImplementation";
import { getPackageManagerVersion } from "../getPackageManagerVersion";
import fs from "fs-extra";
import chalk from "chalk";
import { skip, ok, error } from "../logger";

export default function addPackageManager(files: string[], flags: Flags) {
  if (files.length === 1) {
    const dir = files[0];
    const root = path.resolve(process.cwd(), dir);
    console.log(`Set "packageManager" key in root "package.json" file...`);
    const packageManager = getWorkspaceImplementation(root);
    if (!packageManager) {
      error(`Unable to determine package manager for ${dir}`);
      process.exit(1);
    }
    // handle workspaces...
    const version = getPackageManagerVersion(packageManager);
    const pkgManagerString = `${packageManager}@${version}`;
    const rootPackageJsonPath = path.join(root, "package.json");
    const rootPackageJson = fs.readJsonSync(rootPackageJsonPath);
    const allWorkspaces = [
      {
        name: "package.json",
        path: root,
        packageJson: {
          ...rootPackageJson,
          packageJsonPath: rootPackageJsonPath,
        },
      },
    ];

    let modifiedCount = 0;
    let skippedCount = 0;
    let errorCount = 0;
    let unmodifiedCount = allWorkspaces.length;
    console.log(`Found ${unmodifiedCount} files for modification...`);
    for (const workspace of allWorkspaces) {
      const { packageJsonPath, ...pkgJson } = workspace.packageJson;
      const relPackageJsonPath = path.relative(root, packageJsonPath);
      try {
        if (pkgJson.packageManager === pkgManagerString) {
          skip(
            relPackageJsonPath,
            chalk.dim(`(already set to ${pkgManagerString})`)
          );
        } else {
          const newJson = { ...pkgJson, packageManager: pkgManagerString };
          if (flags.print) {
            console.log(JSON.stringify(newJson, null, 2));
          }
          if (!flags.dry) {
            fs.writeJsonSync(packageJsonPath, newJson, {
              spaces: 2,
            });

            ok(relPackageJsonPath);
            modifiedCount++;
            unmodifiedCount--;
          } else {
            skip(relPackageJsonPath, chalk.dim(`(dry run)`));
          }
        }
      } catch (err) {
        console.error(error);
        error(relPackageJsonPath);
      }
    }
    console.log("All done.");
    console.log("Results:");
    console.log(chalk.red(`${errorCount} errors`));
    console.log(chalk.yellow(`${skippedCount} skipped`));
    console.log(chalk.yellow(`${unmodifiedCount} unmodified`));
    console.log(chalk.green(`${modifiedCount} modified`));
  }
}
