import { TransformerArgs } from "../types";
import type { Schema as TurboJsonSchema } from "@turbo/types";
import { TransformerResults } from "../runner";
import path from "path";
import fs from "fs-extra";
import getTransformerHelpers from "../utils/getTransformerHelpers";
import { getTurboConfigs } from "@turbo/utils";
import Logger from "../utils/logger";

// transformer details
const TRANSFORMER = "clean-globs";
const DESCRIPTION =
  "Automatically clean up invalid globs from your 'turbo.json' file";
const INTRODUCED_IN = "1.11.0";

export function transformer({
  root,
  options,
}: TransformerArgs): TransformerResults {
  const { log, runner } = getTransformerHelpers({
    transformer: TRANSFORMER,
    rootPath: root,
    options,
  });

  const turboConfigPath = path.join(root, "turbo.json");

  const turboJson: TurboJsonSchema = fs.readJsonSync(turboConfigPath);
  runner.modifyFile({
    filePath: turboConfigPath,
    after: migrateConfig(turboJson, log),
  });

  // find and migrate any workspace configs
  const workspaceConfigs = getTurboConfigs(root);
  workspaceConfigs.forEach((workspaceConfig) => {
    const { config, turboConfigPath } = workspaceConfig;
    runner.modifyFile({
      filePath: turboConfigPath,
      after: migrateConfig(config, log),
    });
  });

  return runner.finish();
}

function migrateConfig(config: TurboJsonSchema, log: Logger) {
  const mapGlob = (glob: string) => fixGlobPattern(glob, log);
  for (const [_, taskDef] of Object.entries(config.pipeline)) {
    taskDef.inputs = taskDef.inputs?.map(mapGlob);
    taskDef.outputs = taskDef.outputs?.map(mapGlob);
  }

  return config;
}

export function fixGlobPattern(pattern: string, log: Logger): string {
  let oldPattern = pattern;
  // For '../../app-store/**/**' and '**/**/result.json'
  // Collapse back-to-back doublestars '**/**' to a single doublestar '**'
  let newPattern = pattern.replace(/\*\*\/\*\*/g, "**");
  while (newPattern !== pattern) {
    pattern = newPattern;
    newPattern = pattern.replace(/\*\*\/\*\*/g, "**");
  }

  // For '**.ext' change to '**/*.ext'
  // 'ext' is a filename or extension and can contain almost any character except '*' and '/'
  newPattern = pattern.replace(/(\*\*)([^*/]+)/g, "$1/*$2");
  if (newPattern !== pattern) {
    pattern = newPattern;
  }

  // For 'prefix**' change to 'prefix*/**'
  // 'prefix' is a folder name and can contain almost any character except '*' and '/'
  newPattern = pattern.replace(/([^*/]+)(\*\*)/g, "$1*/$2");
  if (newPattern !== pattern) {
    pattern = newPattern;
  }

  return pattern;
}

const transformerMeta = {
  name: `${TRANSFORMER}: ${DESCRIPTION}`,
  value: TRANSFORMER,
  introducedIn: INTRODUCED_IN,
  transformer,
};

export default transformerMeta;
