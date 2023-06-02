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
  "Automatically clean up invalid globs from your &#x60;turbo.json&#x60; file";
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
    log.modified(`${pattern} to ${newPattern}`);
    pattern = newPattern;
    newPattern = pattern.replace(/\*\*\/\*\*/g, "**");
  }

  // For '!**/dist' and '!**/node_modules'
  // Change '!**/dist' and '!**/node_modules' to '**/[!dist]' and '**/[!node_modules]'
  newPattern = pattern.replace(/^!\*\*\/([a-z_\/]+)$/g, "**/[!$1]");
  if (newPattern !== pattern) {
    log.modified(`${pattern} to ${newPattern}`);
    log.info("please make the previous transform is correct");
    pattern = newPattern;
  }

  // For 'cypress/integration/**.test.ts', 'scripts/**.mjs', 'scripts/**.js', 'src/types/generated/**.ts'
  // Change '**.ext' to '**/*.ext' where 'ext' is 'test.ts', 'mjs', 'js', 'ts'
  newPattern = pattern.replace(/(\*\*)([a-z\.]+)/g, "$1/*$2");
  if (newPattern !== pattern) {
    log.modified(`${pattern} to ${newPattern}`);
    pattern = newPattern;
  }

  // For 'test/prefix**' change to 'test/prefix*/**'
  newPattern = pattern.replace(/([a-z_]+)(\*\*)/g, "$1*/$2");
  if (newPattern !== pattern) {
    log.modified(`${pattern} to ${newPattern}`);
    pattern = newPattern;
  }

  if (oldPattern === pattern) {
    log.unchanged(pattern);
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
