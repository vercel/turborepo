import path from "path";
import fs from "fs-extra";
import chalk from "chalk";
import { logger } from "@turbo/utils";
import { gatherAddRequirements } from "../utils/gatherAddRequirements";
import type { TurboGeneratorArguments } from "./types";
import type { PackageJson, DependencyGroups } from "../types";

export async function generate({ project, opts }: TurboGeneratorArguments) {
  const { name, location, dependencies } = await gatherAddRequirements({
    project,
    opts,
  });

  const packageJson: PackageJson = {
    name,
    version: "0.0.0",
    private: true,
    scripts: {
      build: "turbo build",
    },
  };

  // update dependencies
  Object.keys(dependencies).forEach((group) => {
    const deps = dependencies[group as keyof DependencyGroups];
    if (deps && Object.keys(deps).length > 0) {
      packageJson[group as keyof DependencyGroups] = deps;
    }
  });

  // write the directory
  fs.mkdirSync(location.absolute, { recursive: true });

  // create package.json
  fs.writeFileSync(
    path.join(location.absolute, "package.json"),
    JSON.stringify(packageJson, null, 2)
  );

  // create README
  fs.writeFileSync(path.join(location.absolute, "README.md"), `# \`${name}\``);

  console.log();
  console.log(
    `${chalk.bold(logger.turboGradient(">>> Success!"))} Created ${name} at "${
      location.relative
    }"`
  );
}
