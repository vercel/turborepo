import path from "node:path";
import type { Schema as TurboJsonSchema } from "@turbo/types";
import { readJsonSync } from "fs-extra";
import { getTurboConfigs } from "@turbo/utils";
import type { TransformerArgs } from "../types";
import type { TransformerResults } from "../runner";
import { getTransformerHelpers } from "../utils/getTransformerHelpers";

// transformer details
const TRANSFORMER = "clean-globs";
const DESCRIPTION =
  "Automatically clean up invalid globs from your 'turbo.json' file";
const INTRODUCED_IN = "1.11.0";

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  const turboConfigPath = path.join(root, "turbo.json");

  const turboJson = readJsonSync(turboConfigPath) as TurboJsonSchema;
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateConfig(turboJson),
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  workspaceConfigs.forEach((workspaceConfig) => {
    const { config, turboConfigPath: filePath } = workspaceConfig;
    runner.modifyFile({
      filePath,
      after: migrateConfig(config),
    });
  });

  return runner.finish();
}

function migrateConfig(config: TurboJsonSchema) {
  const mapGlob = (glob: string) => fixGlobPattern(glob);
  for (const [_, taskDef] of Object.entries(config.pipeline)) {
    taskDef.inputs = taskDef.inputs?.map(mapGlob);
    taskDef.outputs = taskDef.outputs?.map(mapGlob);
  }

  return config;
}

export function fixGlobPattern(pattern: string): string {
  let oldPattern = pattern;
  // For '../../app-store/**/**' and '**/**/result.json'
  // Collapse back-to-back doublestars '**/**' to a single doublestar '**'
  let newPattern = oldPattern.replace(/\*\*\/\*\*/g, "**");
  while (newPattern !== oldPattern) {
    oldPattern = newPattern;
    newPattern = oldPattern.replace(/\*\*\/\*\*/g, "**");
  }

  // For '**.ext' change to '**/*.ext'
  // 'ext' is a filename or extension and can contain almost any character except '*' and '/'
  // eslint-disable-next-line prefer-named-capture-group -- using positional replace
  newPattern = oldPattern.replace(/(\*\*)([^*/]+)/g, "$1/*$2");
  if (newPattern !== oldPattern) {
    oldPattern = newPattern;
  }

  // For 'prefix**' change to 'prefix*/**'
  // 'prefix' is a folder name and can contain almost any character except '*' and '/'
  // eslint-disable-next-line prefer-named-capture-group -- using positional replace
  newPattern = oldPattern.replace(/([^*/]+)(\*\*)/g, "$1*/$2");
  if (newPattern !== oldPattern) {
    oldPattern = newPattern;
  }

  return oldPattern;
}

const transformerMeta = {
  name: `${TRANSFORMER}: ${DESCRIPTION}`,
  value: TRANSFORMER,
  introducedIn: INTRODUCED_IN,
  transformer,
};

// eslint-disable-next-line import/no-default-export -- transforms require default export
export default transformerMeta;
