import path from "node:path";
import { writeFileSync, mkdirSync } from "fs-extra";
import { bold } from "picocolors";
import { logger, type PackageJson, type DependencyGroups } from "@turbo/utils";
import { gatherAddRequirements } from "../utils/gatherAddRequirements";
import type { TurboGeneratorArguments } from "./types";

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
      dev: "echo 'Add dev script here'",
      build: "echo 'Add build script here'",
      test: "echo 'Add test script here'",
      lint: "echo 'Add lint script here'",
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
  mkdirSync(location.absolute, { recursive: true });

  // create package.json
  writeFileSync(
    path.join(location.absolute, "package.json"),
    JSON.stringify(packageJson, null, 2)
  );

  // create README
  writeFileSync(path.join(location.absolute, "README.md"), `# \`${name}\``);

  logger.log();
  logger.log(
    `${bold(logger.turboGradient(">>> Success!"))} Created ${name} at "${
      location.relative
    }"`
  );
}
