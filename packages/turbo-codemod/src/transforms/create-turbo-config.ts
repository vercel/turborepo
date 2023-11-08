import path from "node:path";
import { readJsonSync, existsSync } from "fs-extra";
import { type PackageJson } from "@turbo/utils";
import type { Schema } from "@turbo/types";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";
import type { TransformerArgs } from "../types";

// transformer details
const TRANSFORMER = "create-turbo-config";
const DESCRIPTION =
  'Create the `turbo.json` file from an existing "turbo" key in `package.json`';
const INTRODUCED_IN = "1.1.0";

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  log.info(`Migrating "package.json" "turbo" key to "turbo.json" file...`);
  const turboConfigPath = path.join(root, "turbo.json");
  const rootPackageJsonPath = path.join(root, "package.json");
  if (!existsSync(rootPackageJsonPath)) {
    return runner.abortTransform({
      reason: `No package.json found at ${root}. Is the path correct?`,
    });
  }

  // read files
  const rootPackageJson = readJsonSync(rootPackageJsonPath) as PackageJson;
  let rootTurboJson = null;
  try {
    rootTurboJson = readJsonSync(turboConfigPath) as Schema;
  } catch (err) {
    rootTurboJson = null;
  }

  // modify files
  let transformedPackageJson = rootPackageJson;
  let transformedTurboConfig = rootTurboJson;
  if (!rootTurboJson && rootPackageJson.turbo) {
    const { turbo: turboConfig, ...remainingPkgJson } = rootPackageJson;
    transformedTurboConfig = turboConfig;
    transformedPackageJson = remainingPkgJson;
  }

  runner.modifyFile({
    filePath: turboConfigPath,
    after: transformedTurboConfig,
  });
  runner.modifyFile({
    filePath: rootPackageJsonPath,
    after: transformedPackageJson,
  });

  return runner.finish();
}

const transformerMeta = {
  name: TRANSFORMER,
  description: DESCRIPTION,
  introducedIn: INTRODUCED_IN,
  transformer,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
